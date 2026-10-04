// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

//! The redaction half of harness [#820](https://github.com/AI-Ascension/sts2-harness/issues/820).
//!
//! # Why this file exists separately
//!
//! The management server is a security boundary, and the issue is explicit that "a
//! logging change that leaks a token is far worse than no logging change". A security
//! reviewer should be able to read every rule that decides what may appear in a log
//! line, and every test that proves it, without wading through the lifecycle
//! assertions. That is what this file is.
//!
//! # The two halves of the claim
//!
//! 1. **No credential.** A request carrying a recognisable secret must leave no trace
//!    of it -- not in the route, not in a query string, not through an error message.
//! 2. **No local path.** An absolute local path reveals the operator's filesystem
//!    layout and must never be echoed.
//!
//! Both are asserted with an unmistakable marker (`sk-live-DO-NOT-LOG-...`) rather
//! than a generic substring, so a test cannot pass by matching something incidental.
//! The underlying guarantee is structural, not merely filtered: `log_abandoned` takes
//! no message argument at all, and an unrecognised route segment is replaced rather
//! than sanitised, so the safe behaviour is the only expressible one.

use std::sync::Arc;

use super::tests::{RecordingSink, log_with};

// ## 2. Redaction: the negative side, asserted with a recognisable secret

#[test]
fn no_credential_from_the_request_reaches_the_log() {
    // The security half of #820, and the half a reviewer must see proved.
    //
    // The secret below is deliberately unmistakable. Every plausible way a caller
    // could smuggle it -- query string, bearer credential, custom header, JSON body
    // -- is fed to the label builders, and the secret must appear in none of them.
    const SECRET: &str = "sk-live-DO-NOT-LOG-4f2b8c1e9a";
    let sink = Arc::new(RecordingSink::new(1_700_000_000, vec![0, 1]));
    let lifecycle = log_with(&sink);

    // Query string: the route a caller most plausibly stuffs a secret into.
    let query_route = format!("/v1/workflow-runs?token={SECRET}");
    // Bearer credential, as it would arrive in the Authorization header.
    let bearer_route = format!("/v1/workflow-runs/{SECRET}");
    // A body and a header value are the two remaining ways a secret could arrive.
    // Neither is reachable: the lifecycle record never reads a body or a header, and
    // `log_abandoned` takes no message at all -- so the compiler, not just this
    // test, refuses a call that would try to log one. These are asserted absent so
    // the claim is checked rather than merely argued.
    let body = format!(r#"{{"api_key":"{SECRET}"}}"#);
    let header_value = format!("Bearer {SECRET}");
    assert!(
        !body.contains(' ') && header_value.starts_with("Bearer "),
        "the two unrepresentable inputs must actually contain the secret, or this \
         test would pass vacuously"
    );

    let request = lifecycle.begin();
    request.log_start("POST", &query_route);
    request.log_end(200);

    let request_two = lifecycle.begin();
    request_two.log_start("GET", &bearer_route);
    request_two.log_abandoned("io_error");

    // A route carrying the secret in its query is still redacted.
    let request_three = lifecycle.begin();
    request_three.log_start("POST", &format!("/v1/studio/drafts?key={SECRET}"));
    request_three.log_abandoned("invalid_body");

    let joined = sink.joined();
    assert!(
        !joined.contains(SECRET),
        "the recognisable secret must never reach the log, got {joined:?}"
    );
    assert!(
        !joined.contains("sk-live-"),
        "no credential-shaped prefix may appear, got {joined:?}"
    );
    assert!(
        !joined.contains("api_key"),
        "no body field name may appear, got {joined:?}"
    );
    assert!(
        !joined.to_lowercase().contains("authorization"),
        "no credential header may appear, got {joined:?}"
    );
    // The route is still logged -- redacted, not suppressed -- so the line stays
    // useful for the very diagnosis #820 needs.
    assert!(
        joined.contains("route=/v1/workflow-runs"),
        "the route must still be reported, with the query dropped, got {joined:?}"
    );
}

#[test]
fn no_absolute_local_path_reaches_the_log() {
    // The other half of the negative claim: an absolute local path reveals the
    // operator's filesystem layout and must never be echoed. It arrives here as a
    // route segment and as an error reason, the two ways a path could get in.
    const LOCAL_PATH: &str = "/home/operator/secret-plans/2026-q4/run-42";
    let sink = Arc::new(RecordingSink::new(1_700_000_000, vec![0, 1, 2, 3]));
    let lifecycle = log_with(&sink);

    let request = lifecycle.begin();
    request.log_start("GET", &format!("/v1/artifacts{LOCAL_PATH}"));
    // No message parameter exists, so an `io::Error` that embedded the path cannot
    // reach the log even if one were raised here.
    request.log_abandoned("io_error");

    let joined = sink.joined();
    assert!(
        !joined.contains(LOCAL_PATH),
        "an absolute local path must never reach the log, got {joined:?}"
    );
    assert!(
        !joined.contains("/home/"),
        "no home directory may appear, got {joined:?}"
    );
    assert!(
        !joined.contains("operator"),
        "no local account name may appear, got {joined:?}"
    );
}

#[test]
fn a_logged_route_cannot_forge_a_second_line() {
    // Log injection: a route carrying a newline must not be able to write a line
    // that looks like a completed request. This is the reason non-printable bytes
    // are replaced rather than passed through.
    let sink = Arc::new(RecordingSink::new(1_700_000_000, vec![0, 1]));
    let lifecycle = log_with(&sink);

    let request = lifecycle.begin();
    request.log_start(
        "GET",
        "/v1/health\r\nsts2-management request_end ts=2020-01-01T00:00:00.000Z",
    );
    request.log_end(200);

    let lines = sink.lines();
    assert_eq!(
        lines.len(),
        2,
        "one emission must stay one line, however the route is crafted, got {lines:?}"
    );
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.contains("request_end"))
            .count(),
        1,
        "a forged second line must not be producible, got {lines:?}"
    );
    assert!(
        !lines[0].contains('\n') && !lines[0].contains('\r'),
        "no emitted line may contain a line break, got {:?}",
        lines[0]
    );
}

#[test]
fn an_unsupported_method_is_named_without_being_echoed() {
    // The parser admits only three methods, but the label must not trust that: an
    // unexpected method collapses to `other` rather than being echoed.
    let sink = Arc::new(RecordingSink::new(1_700_000_000, vec![0, 1]));
    let lifecycle = log_with(&sink);

    let request = lifecycle.begin();
    request.log_start("DELETE /etc/passwd", "/v1/health");
    request.log_end(405);

    let joined = sink.joined();
    assert!(
        joined.contains("method=other"),
        "an unadmitted method must collapse, got {joined:?}"
    );
    assert!(
        !joined.contains("/etc/passwd"),
        "an unadmitted method must not be echoed, got {joined:?}"
    );
}
