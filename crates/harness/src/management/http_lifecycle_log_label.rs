// SPDX-License-Identifier: MIT

//! Field labelling and redaction for the management request-lifecycle log.
//!
//! Split out of `http_lifecycle_log` because "what may appear in a line" is a
//! distinct concern from "when a line is emitted", and because the redaction rules
//! are the part a security review has to read in isolation.
//!
//! # The rule
//!
//! The management server is a security boundary, so nothing a caller controls may
//! reach the log verbatim. Three properties enforce that, and each is asserted in
//! `http_lifecycle_log_tests.rs`:
//!
//! 1. **Closed vocabulary.** Only path segments the server actually serves are
//!    logged. Anything else becomes `/?`, so a credential pasted into a route -- and
//!    any absolute local path, which always contains segments outside the list -- is
//!    unrecoverable from the line.
//! 2. **No query string.** It is caller-controlled and can carry a secret, so only
//!    the path is logged.
//! 3. **Bounded, single-line labels.** Every label is byte-capped and stripped of
//!    non-printable bytes, so a caller can neither drive unbounded log volume nor
//!    forge a second log line.

use super::MAX_ROUTE_BYTES;

/// Reduce an HTTP method to a bounded, loggable token.
///
/// The server already admits only `GET`, `POST` and `PUT`; anything else is reported
/// as `other` rather than echoed, so a method can never carry caller bytes.
pub(super) fn method_label(method: &str) -> String {
    if matches!(method, "GET" | "POST" | "PUT") {
        format!("method={method}")
    } else {
        String::from("method=other")
    }
}

/// The route vocabulary the management server actually serves.
///
/// Closed on purpose. A segment that is not in this list is not a route this server
/// has, so it is not logged verbatim: that is what stops a secret pasted into a
/// path -- and any absolute local path, which always contains segments outside the
/// list -- from reaching the log at all.
const KNOWN_ROUTE_SEGMENTS: [&str; 22] = [
    "v1",
    "health",
    "workflow-definitions",
    "workflow-runs",
    "workflow-targets",
    "context-bindings",
    "inference-profiles",
    "capabilities",
    "studio",
    "definitions",
    "drafts",
    "authoring-inference",
    "memory-policy-owner",
    "runs",
    "targets",
    "profiles",
    "revise",
    "publish",
    "history",
    "catalog",
    "preflight",
    "bind",
];

/// Reduce a request target to a bounded, loggable `route=` label.
///
/// The parser has already admitted a `/`-prefixed path with no `..`, `\`, `@`, `#`,
/// `%` or `//`. On top of that this applies the properties the log needs: the query
/// string is dropped, every segment is checked against [`KNOWN_ROUTE_SEGMENTS`], and
/// the result is byte-bounded.
pub(super) fn route_label(target: &str) -> String {
    let path = target.split('?').next().unwrap_or(target);
    let mut label = String::from("route=");
    for (index, segment) in path.split('/').enumerate() {
        // The leading empty segment is the root slash; there is nothing to validate.
        if index == 0 {
            if !segment.is_empty() {
                label.push('?');
            }
            continue;
        }
        if !KNOWN_ROUTE_SEGMENTS.contains(&segment) {
            // Unknown segment: `/` plus `?` in its place. Keeping the separator means
            // the segment *count* survives, which is what makes the line still
            // diagnosable, while the `?` makes the content unrecoverable, so neither a
            // credential nor a local path can be reconstructed from it.
            label.push_str("/?");
        } else {
            label.push('/');
            label.push_str(segment);
        }
        if label.len() >= MAX_ROUTE_BYTES {
            label.push_str("..truncated");
            return label;
        }
    }
    label
}

/// Reduce a typed error code to a bounded, loggable token.
///
/// Codes are harness-owned constants, so this is a belt-and-braces bound rather than
/// trust: an unexpected value is still clamped and stripped.
pub(super) fn code_label(code: &str) -> String {
    bounded_label("code=", code)
}

/// The abandonment `reason=` field.
///
/// Deliberately the same bounded token as `code=`, not the error's message: an
/// `io::Error` string can carry caller bytes, and `reason=` must never become a
/// second channel for them. Two identical fields are cheaper than a leak.
pub(super) fn reason_label(code: &str) -> String {
    bounded_label("reason=", code)
}

/// Clamp `text` into a single-line label with a bounded byte length.
fn bounded_label(prefix: &str, text: &str) -> String {
    let mut label = String::from(prefix);
    for character in text.chars() {
        if character.is_ascii_graphic() {
            label.push(character);
        } else {
            // Replaced, not skipped: skipping would silently join two segments into
            // one misleading route. A space cannot forge a new field boundary.
            label.push(' ');
        }
        if label.len() >= MAX_ROUTE_BYTES {
            label.push_str("..truncated");
            break;
        }
    }
    label
}
