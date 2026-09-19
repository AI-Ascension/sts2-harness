// SPDX-License-Identifier: MIT

//! Bounded reporting of why a supervised child produced no answer.
//!
//! Three seams spawn an operator-owned executable with the environment cleared but for a short
//! inherited-name allowlist: the provider transport, the one-shot lifecycle effect, and the
//! long-lived lookup supervisor. In all three, a child that could not start -- a missing
//! interpreter, a wrong path, a rejected interpreter policy -- was indistinguishable from a peer
//! that was down, because the child's standard error, the one channel that names the cause, was
//! sent to the null device.
//!
//! The shared shape is: capture the child's standard error, keep a bounded tail of it, and, when the
//! exchange fails, write one line naming the child's exit and that tail to the harness's own
//! standard error, which is the operator log the runtime already collects. The error vocabularies of
//! the three seams, their wire shapes and their durable records are untouched, and no free-form
//! child text enters any of them.
//!
//! The seams differ in when the stream can be read, so they differ in which helper they call. Both
//! one-shot seams terminate the child and then read what it already wrote, through
//! [`report_stopped_child_failure`]; the duplex supervisor drains the stream while the child runs,
//! because a bridge that writes to a pipe nobody reads blocks on it while the protocol loop waits
//! for a response.
//!
//! A seam that is about to stop its child first asks [`settle_child`] whether the child got there on
//! its own, so an operator line can tell a child that failed from one the harness had to stop. That
//! probe cannot be a plain `try_wait`: a child closes its pipes as it exits and can be observed
//! through them before it is reapable, and reading that window as "still running" would report an
//! expired deadline where the child named its own cause.

use std::process::ExitStatus;
use std::sync::Mutex;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Child;

/// Bytes of a child's standard error kept from the end of the stream.
pub(crate) const MAX_STDERR_BYTES: usize = 2_048;

/// Characters of the escaped tail that reach the operator line.
const MAX_REPORTED_STDERR_BYTES: usize = 256;

/// The grace a child that failed the exchange is given to reach its own exit status.
///
/// The pipe half of an exit and the process half are not the same event: a child that has already
/// closed its standard output can be observed through that end while it is still finishing, so the
/// probe has to wait for the bound rather than sample once. It only elapses for a child that is
/// still running, which is exactly the child the harness has to stop.
pub(crate) const CHILD_SETTLE_MILLIS: u64 = 50;

/// The grace a failed exchange spends reading what a dead child already wrote to its standard error.
///
/// The bytes are usually buffered, so this returns as soon as the pipe reaches end of file. It only
/// elapses when a descendant inherited the child's standard error and still holds it open, which is
/// exactly the case an operator cannot wait for.
pub(crate) const STDERR_TAIL_MILLIS: u64 = 50;

/// Describe a failed exchange for the operator log, or `None` when there is nothing to report.
///
/// `own_exit` is the status the child reached on its own, as [`settle_child`] reports it. A child the
/// harness had to stop is ordinary operation -- a cancelled turn, an expired deadline -- and is
/// reported only when it wrote to the stream, because that is the evidence it left about why it
/// stalled; a child that stopped by itself is reported with the status it stopped with.
pub(crate) fn stopped_failure_line(
    subject: &str,
    own_exit: Option<ExitStatus>,
    tail: &[u8],
) -> Option<String> {
    match own_exit {
        Some(exit) if exit.success() => None,
        Some(exit) => Some(with_tail(&format!("{subject} failed: {exit}"), tail)),
        None if tail.is_empty() => None,
        None => Some(with_tail(
            &format!("{subject} failed before its child exited"),
            tail,
        )),
    }
}

/// Describe a child that could not be spawned at all.
///
/// A spawn failure leaves no child and therefore no standard error: the configured executable and
/// the operating system's own message are the only causes there are, and together they name a wrong
/// path or a rejected launch directly.
pub(crate) fn start_failure_line(
    subject: &str,
    executable: &str,
    error: &std::io::Error,
) -> String {
    format!(
        "{subject} could not start: {}: {}",
        escape_bounded(executable.as_bytes()),
        escape_bounded(error.to_string().as_bytes())
    )
}

/// Wait, within the settle grace, for a failed child to reach an exit status of its own.
///
/// `Some` is the status the child reached by itself before the harness stopped it; `None` means it
/// was still running when the grace expired, so the seam that kills it is reporting its own
/// cleanup rather than the child's cause.
pub(crate) async fn settle_child(child: &mut Child) -> Option<ExitStatus> {
    match tokio::time::timeout(Duration::from_millis(CHILD_SETTLE_MILLIS), child.wait()).await {
        Ok(Ok(status)) => Some(status),
        Ok(Err(_)) | Err(_) => None,
    }
}

/// Report a one-shot child the seam had to stop, reading what it left on its standard error.
///
/// The child is already dead or on its way out, so this is the last moment its standard error can be
/// read: the tail is usually buffered and the read reaches end of file at once. `subject` names the
/// seam, so the one operator line says which child asked for it.
pub(crate) async fn report_stopped_child_failure(
    subject: &str,
    child: &mut Child,
    own_exit: Option<ExitStatus>,
) {
    let Some(stderr) = child.stderr.take() else {
        return;
    };
    let tail = tokio::time::timeout(
        Duration::from_millis(STDERR_TAIL_MILLIS),
        read_tail(stderr, MAX_STDERR_BYTES),
    )
    .await
    .unwrap_or_default();
    if let Some(reason) = stopped_failure_line(subject, own_exit, &tail) {
        eprintln!("{reason}");
    }
}

/// Read a standard error stream to end of file, keeping at most `maximum` bytes from the end.
///
/// A read failure returns what was already read: the reason is worth reporting even when the stream
/// ends early.
async fn read_tail(mut source: impl AsyncRead + Unpin, maximum: usize) -> Vec<u8> {
    let mut tail = Vec::new();
    let mut chunk = [0_u8; 256];
    loop {
        match source.read(&mut chunk).await {
            Ok(0) | Err(_) => return tail,
            Ok(count) => {
                tail.extend_from_slice(&chunk[..count]);
                let excess = tail.len().saturating_sub(maximum);
                tail.drain(..excess);
            }
        }
    }
}

/// Drain a running child's standard error into `tail`, keeping at most `maximum` bytes from the end.
///
/// The stream is read for as long as the child holds it open so that a bridge which writes to its
/// standard error cannot block on a full pipe while the protocol loop waits for its response.
pub(crate) async fn drain_tail(
    mut source: impl AsyncRead + Unpin,
    maximum: usize,
    tail: &Mutex<Vec<u8>>,
) {
    let mut chunk = [0_u8; 256];
    loop {
        match source.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(count) => {
                if let Ok(mut tail) = tail.lock() {
                    tail.extend_from_slice(&chunk[..count]);
                    let excess = tail.len().saturating_sub(maximum);
                    tail.drain(..excess);
                }
            }
        }
    }
}

fn with_tail(head: &str, tail: &[u8]) -> String {
    let tail = escape_bounded(tail);
    let tail = if tail.is_empty() {
        "(empty)"
    } else {
        tail.as_str()
    };
    format!("{head}; stderr tail: {tail}")
}

/// Render bytes as one bounded line of printable text.
fn escape_bounded(bytes: &[u8]) -> String {
    let escaped: String = String::from_utf8_lossy(bytes).escape_debug().collect();
    if escaped.chars().count() <= MAX_REPORTED_STDERR_BYTES {
        return escaped;
    }
    let mut bounded: String = escaped.chars().take(MAX_REPORTED_STDERR_BYTES).collect();
    bounded.push_str("...");
    bounded
}

#[cfg(all(test, unix))]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use std::os::unix::process::ExitStatusExt;

    fn status(code: i32) -> ExitStatus {
        ExitStatus::from_raw(code << 8)
    }

    #[test]
    fn a_child_that_stopped_by_itself_is_reported_with_its_status() {
        assert_eq!(
            stopped_failure_line("seam", Some(status(7)), b"no interpreter\n"),
            Some(String::from(
                "seam failed: exit status: 7; stderr tail: no interpreter\\n"
            ))
        );
    }

    #[test]
    fn a_successful_child_is_not_reported() {
        assert_eq!(
            stopped_failure_line("seam", Some(status(0)), b"noise"),
            None
        );
    }

    #[test]
    fn a_child_the_harness_stopped_is_reported_only_when_it_wrote_something() {
        assert_eq!(stopped_failure_line("seam", None, b""), None);
        assert_eq!(
            stopped_failure_line("seam", None, b"stuck"),
            Some(String::from(
                "seam failed before its child exited; stderr tail: stuck"
            ))
        );
    }

    #[test]
    fn a_spawn_failure_names_the_message_it_was_given() {
        let error = std::io::Error::new(std::io::ErrorKind::NotFound, "No such file or directory");
        assert_eq!(
            start_failure_line("seam", "/bridge", &error),
            String::from("seam could not start: /bridge: No such file or directory")
        );
    }

    #[test]
    fn a_flood_is_reported_as_one_bounded_escaped_line() {
        let mut tail = vec![b'n'; 4_096];
        tail.push(0x1b);
        let line = stopped_failure_line("seam", Some(status(3)), &tail).expect("a report");
        assert!(line.contains("stderr tail: "), "{line}");
        assert!(!line.contains('\u{1b}'), "a raw control byte was written");
        assert!(
            line.chars().count() < 400,
            "the operator line is unbounded: {} characters",
            line.chars().count()
        );
    }
}
