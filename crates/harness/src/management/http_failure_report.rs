// SPDX-License-Identifier: MIT

use super::HttpError;
use std::net::SocketAddr;
use std::sync::Arc;

/// The port the owner reports an undeliverable management response through.
///
/// The report is the only evidence that survives a connection the owner could not answer: the
/// socket is gone, so the peer is told nothing, and a line that is never emitted is indistinguishable
/// from a connection that was never made. Routing the line through a port rather than writing it
/// inline is what makes that evidence assertable — a test can observe the report, where no test can
/// observe a raw `eprintln!` (#819).
pub trait ManagementFailurePort: Send + Sync {
    /// Records one already-rendered operator line.
    fn report(&self, line: &str);

    /// Whether this port records what it is handed.
    fn reports(&self) -> bool;
}

/// The recording port attached to the management server.
///
/// The production composition writes each report to standard error, and that default is the whole
/// point of #816: an owner that computed an answer and could not send it must say so. There is
/// deliberately no inert `disabled()` constructor, because a no-op port would restore exactly the
/// silence this sink exists to remove, and it would do so by a single unremarkable builder call.
/// The only ways to attach a port are the stderr default and an explicit port, so suppressing these
/// reports requires writing a trait implementation that drops the line.
#[derive(Clone)]
pub struct ManagementFailureSink(Arc<dyn ManagementFailurePort>);

impl ManagementFailureSink {
    /// Attaches one recording port.
    pub fn new(port: Arc<dyn ManagementFailurePort>) -> Self {
        Self(port)
    }

    /// The production port: one bounded, attributed line on standard error.
    pub fn stderr() -> Self {
        Self(Arc::new(StderrFailurePort))
    }

    /// Records one line through the attached port.
    pub fn report(&self, line: &str) {
        self.0.report(line);
    }

    /// Whether the attached port records what it is handed.
    ///
    /// Every port in this crate reports, so this is true. It exists so a test can assert that the
    /// *default* sink is a reporting one: the stderr line cannot be observed in-process, but "the
    /// default does not discard" is the property #816 needs and is checkable here.
    pub fn reports_unread_failures(&self) -> bool {
        self.0.reports()
    }

    /// Reports one connection that ended without a delivered response.
    ///
    /// The whole report is assembled here rather than at the call site, so the accept loop stays
    /// a loop and the line's shape is owned by the module that also renders it for tests.
    pub(crate) fn report_connection(
        &self,
        peer: SocketAddr,
        request: Option<&str>,
        error: &HttpError,
    ) {
        self.report(&connection_failure_line(peer, request, error));
    }
}

impl Default for ManagementFailureSink {
    fn default() -> Self {
        Self::stderr()
    }
}

/// The production reporting port.
struct StderrFailurePort;

impl ManagementFailurePort for StderrFailurePort {
    fn report(&self, line: &str) {
        eprintln!("{line}");
    }

    fn reports(&self) -> bool {
        true
    }
}

/// The bounded width of one rendered report field.
///
/// A report is one operator line, not a payload: an over-long cause must not become a log-flooding
/// vector, so every field is escaped and clipped before it is joined.
const MAX_REPORT_FIELD_CHARS: usize = 256;

/// Render the one line that records a management connection which ended without a delivered
/// response.
///
/// `request` is the method and path the owner had already read, or `None` when the request never
/// parsed. The distinction matters: a report that cannot be tied to the request that caused it is
/// much weaker evidence when a run goes red, so the line names the peer and the route rather than
/// restating only the failure (#816, #819).
///
/// Every field is escaped and clipped. The peer is a socket address and the cause is an owner-owned
/// message, so neither carries an environment, a credential, or a local path; the request path is
/// peer-supplied and is therefore treated as untrusted text.
pub(super) fn connection_failure_line(
    peer: SocketAddr,
    request: Option<&str>,
    error: &HttpError,
) -> String {
    let request = request
        .map(|request| escape_bounded(request.as_bytes()))
        .unwrap_or_else(|| UNREAD_REQUEST.to_owned());
    format!(
        "sts2-management connection ended without a delivered response: peer={} request={request} cause={}: {}",
        peer,
        error.code,
        escape_bounded(error.message.as_bytes()),
    )
}

/// What the line says when the owner never got far enough to learn the route.
const UNREAD_REQUEST: &str = "(not read)";

/// Render bytes as one bounded line of printable text.
fn escape_bounded(bytes: &[u8]) -> String {
    let escaped: String = String::from_utf8_lossy(bytes).escape_debug().collect();
    if escaped.chars().count() <= MAX_REPORT_FIELD_CHARS {
        return escaped;
    }
    let mut bounded: String = escaped.chars().take(MAX_REPORT_FIELD_CHARS).collect();
    bounded.push_str("...");
    bounded
}

#[cfg(test)]
#[path = "http_failure_report_tests.rs"]
mod tests;
