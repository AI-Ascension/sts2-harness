// SPDX-License-Identifier: MIT

use std::net::SocketAddr;

/// A connection failure the peer can no longer be told about.
///
/// Once `handle_connection` has failed, the socket is gone and the only place
/// the fact can still go is the process's own stderr. That makes the report
/// write-only and awkward to assert on: a test can capture stderr, but the
/// obvious way to break this -- deleting the `eprintln!` -- passes every other
/// test in the suite, because nothing else observes it. Routing the report
/// through this sink is what makes the line testable at all (harness#819).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UndeliveredResponse {
    /// The peer's address, so a report can be tied to a connection.
    pub peer: SocketAddr,
    /// The management error's stable code, e.g. `deadline_exceeded`.
    pub code: String,
    /// The error's human-readable message.
    pub message: String,
}

/// Where an undelivered-response report goes.
///
/// Production writes to stderr. Tests install a sink so the line is asserted
/// rather than merely reviewed.
pub trait DiagnosticsSink: Send + Sync + 'static {
    fn report(&self, report: &UndeliveredResponse);
}

/// The default sink: one attributed line on stderr.
#[derive(Clone, Copy, Debug, Default)]
pub struct StderrDiagnostics;

impl DiagnosticsSink for StderrDiagnostics {
    fn report(&self, report: &UndeliveredResponse) {
        // Attribution is the whole point of this line: without the peer it is
        // a fact about *a* connection, not about a request, and cannot be tied
        // back to anything when a run goes red (harness#819).
        eprintln!(
            "sts2-management peer={} code={} response not delivered: {}",
            report.peer, report.code, report.message
        );
    }
}
