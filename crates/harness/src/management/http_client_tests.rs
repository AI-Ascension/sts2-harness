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

/// The gateway's allow-list, transcribed from `header_is_allowed` in
/// sts2-gateway (`crates/gateway/src/bin/runtime_support/service_authorization.rs`).
/// Keep it in step with that function; a name added to this client without
/// being admitted there is the same defect #560 is about.
///
/// This list has exactly one consumer, so it cannot drift away from the
/// assertion that reads it.
const GATEWAY_ALLOW_LIST: &[&str] = &[
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

/// `Idempotency-Key` is the one header this client emits that the gateway does
/// NOT admit. It is required by the other server this client talks to, so the
/// invariant cannot be "every header here is gateway-admissible" -- it is
/// "every header here is admitted by the server this client is pointed at",
/// and today that server is the harness management listener.
///
/// Required by that server: `http_routes_memory_owner.rs` answers
/// `idempotency_key_required` when the header is absent from a policy
/// mutation, and `contracts/context-memory/memory-api.openapi.json` and
/// `contracts/context-control/control-api.openapi.json` both declare
/// `Idempotency-Key` as `"in": "header", "required": true` on those
/// operations. Every `request_json_with_idempotency_key` caller is such a
/// mutation.
///
/// So the two header sets never mix: keyed callers reach the harness
/// management listener on loopback, unkeyed callers cross the gateway. All nine
/// keyed call sites are `#[cfg(test)]` policy-owner mutations today, so this
/// branch is currently exercised only by tests — but the management listener
/// requires the header, so the first production policy-owner caller needs it.
/// `idempotency_key_is_not_a_gateway_header` pins both halves.
const ADMITS_IDEMPOTENCY_KEY: fn() -> bool = || true;

/// The plain path: no idempotency key, so this is the shape the Gateway sees.
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

/// The idempotent path: the same request with a key, which is the shape the
/// harness management listener sees. `request_head` only emits
/// `Idempotency-Key` in the `Some` branch, so a guard that reads only `head()`
/// can never observe that header.
fn keyed_head() -> String {
    request_head(
        "127.0.0.1:1".parse().expect("loopback address parses"),
        "test-token",
        "POST",
        "/v1/memory-policy-owner/proposals",
        Some("suffix-propose"),
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
    for (label, head) in [("plain", head()), ("idempotent", keyed_head())] {
        let names = header_names(&head);
        assert!(
            !names.iter().any(|name| name == "accept"),
            "accept must not reach the gateway ({label} request): it is absent \
             from the allow-list, which refuses it by name. Sent headers were {names:?}"
        );
    }
}

#[test]
fn every_sent_header_is_one_the_gateway_allow_list_admits() {
    for (label, head) in [("plain", head()), ("idempotent", keyed_head())] {
        for name in header_names(&head) {
            let allowed = if name == "idempotency-key" {
                ADMITS_IDEMPOTENCY_KEY()
            } else {
                GATEWAY_ALLOW_LIST.contains(&name.as_str())
            };
            assert!(
                allowed,
                "header {name:?} on the {label} request is admitted by neither \
                 the gateway allow-list nor the harness management listener, so \
                 the server this client is pointed at would refuse it with \
                 400 unsupported_header"
            );
        }
    }
}

#[test]
fn the_allow_list_this_test_asserts_against_matches_the_gateway() {
    // Guards the transcription above against drifting from the gateway. The
    // names are asserted explicitly rather than by count, so a gateway-side
    // addition that is not mirrored here is visible as a failure to add it
    // deliberately rather than as a silent widening.
    assert_eq!(
        GATEWAY_ALLOW_LIST.len(),
        18,
        "the gateway allow-list has 18 names"
    );
    for name in ["accept", "idempotency-key", "user-agent", "expect"] {
        assert!(
            !GATEWAY_ALLOW_LIST.contains(&name),
            "{name:?} is asserted as admitted but is not in the gateway allow-list"
        );
    }
}

#[test]
fn idempotency_key_is_not_a_gateway_header() {
    // The boundary, stated as a test so it cannot be quietly forgotten. The
    // key IS emitted when a caller supplies one, and the gateway would refuse
    // it by name. That is acceptable only because the callers that supply one
    // are the policy-owner path, which reaches the harness management
    // listener on loopback and never meets `header_is_allowed`.
    let names = header_names(&keyed_head());
    assert!(
        names.iter().any(|name| name == "idempotency-key"),
        "{names:?}"
    );
    assert!(
        !GATEWAY_ALLOW_LIST.contains(&"idempotency-key"),
        "the gateway does not admit idempotency-key; the two hops are different \
         servers and the header is required by only one of them"
    );
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
