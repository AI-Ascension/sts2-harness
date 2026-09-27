// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

use super::*;

// The contract under test is the exact byte sequence this client writes to the
// socket, because the gateway rejects any header outside its allow-list by
// name. Asserting on the constructed head is therefore the only assertion that
// can catch this class of defect: a request that is "logically the same" but
// carries one extra header is refused in production and accepted here.
//
// Issue #560. The gateway pins `accept` as refused with a test of its own, so
// this is a caller-side fix; the allow-list is unchanged.

fn head() -> String {
    request_head(
        "127.0.0.1:1".parse().expect("loopback address parses"),
        "test-token",
        "POST",
        "/v1/workflows",
        None,
        2,
    )
}

fn header_names(head: &str) -> Vec<String> {
    head.split("\r\n")
        .skip(1)
        .take_while(|line| !line.is_empty())
        .filter_map(|line| line.split_once(':'))
        .map(|(name, _)| name.trim().to_ascii_lowercase())
        .collect()
}

#[test]
fn the_request_head_carries_no_accept_header() {
    let names = header_names(&head());
    assert!(
        !names.iter().any(|name| name == "accept"),
        "accept must not reach the gateway: it is absent from the allow-list, \
         which refuses it by name. Sent headers were {names:?}"
    );
}

#[test]
fn every_sent_header_is_one_the_gateway_allow_list_admits() {
    // The gateway's allow-list, transcribed. Keep this in step with
    // `header_is_allowed` in sts2-gateway; a name added to the client without
    // being admitted there is the same defect this issue is about.
    const ALLOWED: &[&str] = &[
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
        // Admitted by the gateway as a non-gateway header.
        "idempotency-key",
    ];
    for name in header_names(&head()) {
        assert!(
            ALLOWED.contains(&name.as_str()),
            "header {name:?} is not in the gateway allow-list and would be refused \
             with 400 unsupported_header"
        );
    }
}

#[test]
fn the_required_headers_and_the_body_length_are_still_present() {
    let head = head();
    assert!(
        head.starts_with("POST /v1/workflows HTTP/1.1\r\n"),
        "{head}"
    );
    assert!(head.contains("\r\nHost: 127.0.0.1:1\r\n"), "{head}");
    assert!(
        head.contains("\r\nAuthorization: Bearer test-token\r\n"),
        "{head}"
    );
    assert!(
        head.contains("\r\nContent-Type: application/json\r\n"),
        "{head}"
    );
    assert!(head.contains("\r\nContent-Length: 2\r\n"), "{head}");
    assert!(head.contains("\r\nConnection: close\r\n"), "{head}");
    assert!(head.ends_with("\r\n\r\n"), "{head}");
}

#[test]
fn an_idempotency_key_is_still_emitted_when_one_is_supplied() {
    let head = request_head(
        "127.0.0.1:1".parse().expect("loopback address parses"),
        "test-token",
        "POST",
        "/v1/workflows",
        Some("key-42"),
        2,
    );
    assert!(head.contains("\r\nIdempotency-Key: key-42\r\n"), "{head}");
    // The optional segment must not disturb the rest of the head.
    assert!(head.contains("\r\nContent-Length: 2\r\n"), "{head}");
    assert!(head.ends_with("\r\n\r\n"), "{head}");
}
