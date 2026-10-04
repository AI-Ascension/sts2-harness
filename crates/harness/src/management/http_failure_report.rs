// SPDX-License-Identifier: MIT

use super::HttpError;
use std::net::SocketAddr;
use std::sync::Arc;
#[cfg(test)]
use std::sync::Mutex;
use std::sync::RwLock;

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
///
/// The line is written through [`production_writer`] rather than an inline `eprintln!`. That
/// indirection is the seam this module's own test substitutes, and it exists because of #824: the
/// previous probe here asserted a `reports()` predicate hard-coded to `true`, so making the write
/// below a no-op left the entire suite green. The predicate could not fail, so it protected nothing.
/// With the write itself behind one seam, a test can observe the production composition actually
/// emitting rather than observing a constant.
struct StderrFailurePort;

impl ManagementFailurePort for StderrFailurePort {
    fn report(&self, line: &str) {
        (production_writer())(line);
    }
}

/// One place a rendered operator line can be written to.
type ReportWriter = Arc<dyn Fn(&str) + Send + Sync>;

/// Where production reports go, unless a test has temporarily redirected them.
///
/// `None` is the production state and writes real stderr. A test installs a recorder under the
/// test-only lock that serialises substitutions, and restores `None` before releasing that lock, so
/// the substitution cannot outlive the test that made it.
///
/// The lock is named in prose rather than as an intra-doc link because it is `#[cfg(test)]`: a
/// link to it would not resolve in a documented non-test build, which is exactly the kind of
/// documentation defect the `cargo doc` gate exists to catch.
static STDERR_WRITER: RwLock<Option<ReportWriter>> = RwLock::new(None);

/// The writer production reports through right now.
fn production_writer() -> ReportWriter {
    match STDERR_WRITER.read() {
        Ok(writer) => match writer.as_ref() {
            Some(writer) => Arc::clone(writer),
            None => Arc::new(write_to_stderr),
        },
        // A poisoned lock is not a reason to stop reporting: fall back to real stderr, which is
        // the behaviour this whole port exists to preserve.
        Err(_) => Arc::new(write_to_stderr),
    }
}

/// The real writer. One line per report.
fn write_to_stderr(line: &str) {
    eprintln!("{line}");
}

/// Serialises the process-wide writer override between tests that install one.
#[cfg(test)]
pub(super) static STDERR_WRITER_LOCK: Mutex<()> = Mutex::new(());

/// Redirects production reports into `writer` until the caller restores `None`.
///
/// Test-only. Nothing in production calls this, which is why the reset is the caller's job rather
/// than something a guard type owns: the lock is already held for the whole substitution.
#[cfg(test)]
pub(super) fn redirect_production_reports(writer: ReportWriter) -> Result<(), HttpError> {
    let mut slot = STDERR_WRITER.write().map_err(|_| {
        HttpError::new(
            "stderr_writer_poisoned",
            "the stderr writer lock is poisoned",
        )
    })?;
    *slot = Some(writer);
    Ok(())
}

/// Restores production reports to real stderr.
#[cfg(test)]
pub(super) fn restore_production_reports() {
    if let Ok(mut slot) = STDERR_WRITER.write() {
        *slot = None;
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
