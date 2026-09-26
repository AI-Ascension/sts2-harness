// SPDX-License-Identifier: MIT

//! The assertions for [`super`]'s peer-header probe.
//!
//! Split from the probe itself so each file stays under the repository's preferred
//! 400-nonblank budget (`SIZE001`); the probe carries the socket, process, and
//! script handling, and this file carries only what the probe is asked to prove.
//!
//! Every test here drives the *generated* Python script over a real loopback socket, so
//! an assertion is about the bytes `http.client` actually emitted.

use super::super::peer::mcp_server_script;
use super::{
    GATEWAY_ALLOWED_HEADERS, GATEWAY_PATHS, GATEWAY_TOOLS, UNSET_HEADER_SENTINEL, peer_request_for,
    strip_python_comments, temporary_directory,
};

/// The gateway refuses any header outside its closed allow-list, and `http.client`
/// injects `Accept-Encoding: identity` on its own unless told not to. This asserts the
/// injected header is genuinely absent from the bytes the peer emits.
///
/// Non-vacuous: before the fix the peer produced a request carrying exactly one
/// `accept-encoding` header (measured at 10 headers versus 9 after). A test asserting
/// only that the request "succeeds" would have passed both before and after.
#[test]
fn peer_request_carries_no_accept_encoding_header() {
    for tool in GATEWAY_TOOLS {
        let request = peer_request_for(tool);
        let names = request.names();
        assert!(
            !names.iter().any(|name| name == "accept-encoding"),
            "{tool} must not send a header it never asked for; got {names:?}"
        );
        assert_eq!(
            names.iter().filter(|name| *name == "host").count(),
            1,
            "http.client still supplies Host exactly once for {tool}: {names:?}"
        );
        let path = GATEWAY_PATHS
            .iter()
            .find(|(probed, _)| probed == tool)
            .map(|(_, path)| *path)
            .expect("every probed tool declares its request path");
        assert!(
            request.request_line.starts_with(&format!("POST {path} ")),
            "suppression must not disturb the {tool} request line: {}",
            request.request_line
        );
        let declared: usize = request
            .value("content-length")
            .expect("the peer must still declare a body length")
            .parse()
            .expect("content-length is a decimal byte count");
        assert!(
            declared > 0,
            "the {tool} body is non-empty, so a declared length is required"
        );
        assert!(
            request.value("content-type") == Some("application/json"),
            "the scripted content type survives suppression for {tool}: {:?}",
            request.value("content-type")
        );
    }
}

/// Every header the peer actually emits is a member of the gateway's published
/// allow-list, so a future header cannot be introduced into this fixture and left to be
/// discovered as an unattributable 400 in CI.
#[test]
fn every_peer_header_is_on_the_gateway_allow_list() {
    for tool in GATEWAY_TOOLS {
        let request = peer_request_for(tool);
        let names = request.names();
        assert!(
            !names.is_empty(),
            "the {tool} probe must capture a real request, not an empty one"
        );
        for name in &names {
            assert!(
                GATEWAY_ALLOWED_HEADERS.contains(&name.as_str()),
                "{tool} sent header {name:?}, which is not on the gateway allow-list at \
                 cd7e80c6; sent: {names:?}"
            );
        }
    }
}

/// The fix is scoped suppression, not a wider header set: a header the script does not
/// set is still absent, so a later "just allow everything" edit fails here.
#[test]
fn a_header_the_peer_does_not_set_is_still_absent() {
    for tool in GATEWAY_TOOLS {
        let request = peer_request_for(tool);
        let names = request.names();
        assert!(
            !names.iter().any(|name| name == UNSET_HEADER_SENTINEL),
            "{tool}: {UNSET_HEADER_SENTINEL} is allow-listed but never set by the peer; it must \
             stay absent"
        );
        for absent in ["authorization", "x-mcp-request-id", "x-sts2-caller-id"] {
            assert!(
                !names.iter().any(|name| name == absent),
                "{tool}: the peer must not invent {absent:?}; sent: {names:?}"
            );
        }
        assert_eq!(
            names.len(),
            9,
            "exactly the nine scripted/library headers are expected for {tool}; sent: {names:?}"
        );
    }
}

/// The allow-list is a membership test on header *names*, so the value the peer would
/// have sent is irrelevant to the refusal. This records why the value `identity` does
/// not appear anywhere in the generated script, pinning the measured basis for the fix.
#[test]
fn the_peer_script_never_names_accept_encoding_itself() {
    let root = temporary_directory("static");
    let script = root.join("entry-mcp.py");
    let log = root.join("entry-mcp.jsonl");
    let source = mcp_server_script(&log);
    std::fs::write(&script, &source).expect("write generated peer script");
    // Strip comments before searching. The fix's own explanatory comment quotes
    // the rejected form on purpose, and a naive substring search over the raw
    // script reports that comment as the very header the test forbids — the
    // assertion would fail on correct code and pass on the defect it exists to
    // catch. Only executable text can put a header on the wire.
    let code = strip_python_comments(&source).to_ascii_lowercase();
    assert!(
        !code.contains("\"accept-encoding\"") && !code.contains("'accept-encoding'"),
        "the peer must not name accept-encoding as a header value; the gateway refuses the name. \
         Executable text was: {code}"
    );
    assert!(
        code.contains("skip_accept_encoding=true"),
        "the peer must suppress the library default explicitly"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Both call sites route through the shared helper, so a fix applied to only one would
/// leave the other refused. This asserts both are present in the generated bytes.
#[test]
fn both_gateway_call_sites_use_the_header_suppressing_helper() {
    let root = temporary_directory("sites");
    let script = root.join("entry-mcp.py");
    let log = root.join("entry-mcp.jsonl");
    let source = mcp_server_script(&log);
    std::fs::write(&script, &source).expect("write generated peer script");
    let uses = source.matches("post_gateway(").count();
    assert_eq!(
        uses, 3,
        "one definition plus both call sites must route through post_gateway; found {uses}"
    );
    for path in [
        "/game-information/detail",
        "/game-information/live-observation-bootstrap",
    ] {
        assert!(source.contains(path), "the peer must still address {path}");
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// A guard on the embedded allow-list itself: if the gateway's list is edited here, the
/// tests above would silently test a different list. This fails on any change so the
/// edit is a deliberate one against a fresh read of the gateway.
#[test]
fn the_embedded_allow_list_is_the_pinned_gateway_list() {
    assert_eq!(
        GATEWAY_ALLOWED_HEADERS.len(),
        18,
        "service_authorization.rs at cd7e80c6 matches 18 names"
    );
    assert!(
        !GATEWAY_ALLOWED_HEADERS.contains(&"accept-encoding"),
        "accept-encoding is not on the pinned list; that is why the peer must suppress it"
    );
    let mut sorted = GATEWAY_ALLOWED_HEADERS.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        GATEWAY_ALLOWED_HEADERS.len(),
        "the embedded list must not contain duplicates"
    );
    assert!(
        GATEWAY_ALLOWED_HEADERS.contains(&"host"),
        "the library-supplied Host header is allow-listed, which is why suppressing only \
         accept-encoding leaves a fully admitted request"
    );
}
