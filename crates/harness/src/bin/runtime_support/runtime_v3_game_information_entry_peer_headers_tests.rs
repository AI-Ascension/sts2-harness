// SPDX-License-Identifier: MIT

//! Wire-level contract for the synthetic entry peer's Gateway requests.
//!
//! The Gateway's header allow-list is closed, so a peer that lets a client
//! library inject a header it never named is refused with a 400 that says only
//! `unsupported_header`. These tests drive the *generated* script against a
//! loopback listener and assert on the bytes it actually puts on the wire, which
//! is the only place the defect is observable: a comment, or a request that
//! merely "succeeded", cannot distinguish the two caller forms. (Refs #541, #547)

use super::peer::mcp_server_script;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// The Gateway's closed header allow-list, transcribed from
/// `crates/gateway/src/bin/runtime_support/service_authorization.rs`
/// (`header_is_allowed`) at `AI-Ascension/sts2-gateway@cd7e80c6`.
///
/// This is a deliberate maintenance obligation, not a convenience: the Gateway
/// tests the header *name*, so if it widens this list the names above are stale
/// and this test is EXPECTED to fail until they are refreshed. Note there is no
/// `accept-encoding` entry, in that revision or on `main` — which is why the
/// peer must suppress CPython's default rather than set the header itself.
const GATEWAY_HEADER_ALLOW_LIST: &[&str] = &[
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

/// Header names the peer is required to send, beyond the ones `http.client`
/// derives from the request line and body. Asserting these keeps the fix from
/// degenerating into "send less and hope": suppressing the library default must
/// not cost a header the Gateway needs to authorize or route the call.
const REQUIRED_PEER_HEADERS: &[&str] = &[
    "content-type",
    "content-length",
    "x-mcp-session-id",
    "x-sts2-instance-id",
    "x-sts2-session-id",
    "x-sts2-lease-id",
    "x-sts2-lease-epoch",
    "x-sts2-correlation-id",
];

/// A header the script never sets. If a future edit "helpfully" forwards
/// everything, this fails even if the allow-list assertion is widened to match.
const UNSET_HEADER: &str = "x-sts2-caller-id";

struct CapturedRequest {
    header_names: Vec<String>,
    raw: String,
}

/// Tests within one binary run concurrently, so each capture needs its own
/// scratch directory; sharing one would let a test delete another's script.
static CAPTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Start a loopback listener, run the generated peer against it, and return the
/// first request's header names plus the raw request head.
fn capture_peer_request(tool: &str, arguments: &str) -> CapturedRequest {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback listener");
    let address: SocketAddr = listener.local_addr().expect("loopback address");
    let (sender, receiver) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept peer connection");
        let mut head = Vec::new();
        let mut byte = [0_u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            match stream.read(&mut byte) {
                Ok(0) | Err(_) => break,
                Ok(_) => head.push(byte[0]),
            }
        }
        let raw = String::from_utf8_lossy(&head).into_owned();
        // Answer with a body the peer can parse, then let it exit on stdin EOF.
        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}");
        let _ = stream.flush();
        let _ = sender.send(raw);
    });

    let sequence = CAPTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "sts2-entry-peer-headers-{}-{sequence}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("create peer scratch directory");
    let script = root.join("entry-mcp.py");
    let log = root.join("entry-mcp.jsonl");
    std::fs::write(&script, mcp_server_script(&log)).expect("write generated peer script");

    let mut child = Command::new("/usr/bin/python3")
        .arg(&script)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("STS2_GATEWAY_ADDR", address.to_string())
        .env("STS2_SESSION_ID", "session-1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn generated peer script");

    let request = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{{\"name\":\"{tool}\",\"arguments\":{arguments}}}}}\n"
    );
    child
        .stdin
        .as_mut()
        .expect("peer stdin")
        .write_all(request.as_bytes())
        .expect("write tool call to peer");
    // Dropping stdin closes the loop after this single call, so the peer exits.
    drop(child.stdin.take());

    let raw = receiver
        .recv_timeout(Duration::from_secs(30))
        .expect("peer sent a request to the loopback listener");
    let _ = child.wait();
    server.join().expect("join loopback listener");
    let _ = std::fs::remove_dir_all(&root);

    let header_names = raw
        .lines()
        .skip(1)
        .take_while(|line| !line.trim().is_empty())
        .filter_map(|line| line.split_once(':'))
        .map(|(name, _)| name.trim().to_ascii_lowercase())
        .collect();
    CapturedRequest { header_names, raw }
}

/// Arguments accepted by both Gateway-facing call sites. `query_response` takes
/// the `live` branch only when `instance_ref` is present, so it is set here.
const PEER_ARGUMENTS: &str = concat!(
    r#"{"instance_id":"instance-1","mcp_session_id":"mcp-session-1","lease_id":"lease-1","lease_epoch":1,"#,
    r#""definition_ref":{"content_manifest_id":"content-1","entity_kind":"card","namespaced_id":"ironclad:strike"},"#,
    r#""instance_ref":{"content_manifest_id":"content-1","entity_kind":"card","namespaced_id":"ironclad:bash"},"#,
    r#""snapshot_ref":{"state_generation":0},"content_manifest_id":"content-1","locale":"en-US","#,
    r#""namespaced_ids":[],"definition_refs":[],"instance_ids":[],"projection":"summary","#,
    r#""detail_level":"summary","fields":["display_name"],"page_items":4,"item_bytes":4096,"#,
    r#""page_bytes":65536,"text_bytes":4096,"cursor":null}"#
);

fn assert_no_implicit_accept_encoding(captured: &CapturedRequest, call_site: &str) {
    assert!(
        !captured
            .header_names
            .iter()
            .any(|name| name == "accept-encoding"),
        "{call_site} put an `accept-encoding` header on the wire that the script never \
         named, which the Gateway's closed allow-list refuses. Captured request:\n{}",
        captured.raw
    );
}

fn assert_headers_within_allow_list(captured: &CapturedRequest, call_site: &str) {
    for name in &captured.header_names {
        assert!(
            GATEWAY_HEADER_ALLOW_LIST.contains(&name.as_str()),
            "{call_site} sent `{name}`, which is not on the Gateway allow-list at \
             sts2-gateway@cd7e80c6. Captured request:\n{}",
            captured.raw
        );
    }
}

#[test]
fn peer_detail_request_omits_the_implicit_accept_encoding_header() {
    let captured = capture_peer_request("sts2.game_information_detail", PEER_ARGUMENTS);
    assert_no_implicit_accept_encoding(&captured, "game-information/detail");
    assert_headers_within_allow_list(&captured, "game-information/detail");
}

#[test]
fn peer_bootstrap_request_omits_the_implicit_accept_encoding_header() {
    let captured = capture_peer_request(
        "sts2.game_information.live_observation_bootstrap",
        PEER_ARGUMENTS,
    );
    assert_no_implicit_accept_encoding(&captured, "live-observation-bootstrap");
    assert_headers_within_allow_list(&captured, "live-observation-bootstrap");
}

#[test]
fn peer_gateway_requests_still_send_every_header_they_are_supposed_to() {
    for (tool, call_site) in [
        ("sts2.game_information_detail", "game-information/detail"),
        (
            "sts2.game_information.live_observation_bootstrap",
            "live-observation-bootstrap",
        ),
    ] {
        let captured = capture_peer_request(tool, PEER_ARGUMENTS);
        for required in REQUIRED_PEER_HEADERS {
            assert!(
                captured.header_names.iter().any(|name| name == required),
                "{call_site} stopped sending `{required}`; suppressing the client's \
                 default header must not cost a header the Gateway needs. \
                 Captured request:\n{}",
                captured.raw
            );
        }
    }
}

#[test]
fn peer_gateway_requests_do_not_send_headers_the_script_never_sets() {
    for tool in [
        "sts2.game_information_detail",
        "sts2.game_information.live_observation_bootstrap",
    ] {
        let captured = capture_peer_request(tool, PEER_ARGUMENTS);
        assert!(
            !captured
                .header_names
                .iter()
                .any(|name| name == UNSET_HEADER),
            "peer forwarded `{UNSET_HEADER}`, which the script never sets; a \
             \"send everything\" edit must not pass. Captured request:\n{}",
            captured.raw
        );
    }
}

/// Guards the allow-list transcription above against silently drifting out of
/// sync with the Gateway: the embedded list is 18 names at `cd7e80c6`, and if
/// the Gateway widens it this count is the first thing a reviewer must revisit.
#[test]
fn gateway_allow_list_transcription_is_complete() {
    assert_eq!(GATEWAY_HEADER_ALLOW_LIST.len(), 18);
    let mut sorted = GATEWAY_HEADER_ALLOW_LIST.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), GATEWAY_HEADER_ALLOW_LIST.len());
}
