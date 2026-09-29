// SPDX-License-Identifier: MIT

//! #141's fourth acceptance criterion is a privacy guarantee, and it is currently unpinned.
//!
//! `exchange_process` writes the decision request to the child's stdin and nowhere else, so today a
//! request cannot reach argv. That is structural, not enforced: `ExoProcessConfig::new` accepts any
//! `valid_path` string under 2 KiB as an argument, and a request smaller than that is a perfectly
//! valid argument. Only caller convention keeps a prompt fragment out of the command line, where
//! any local user can still read it from `/proc/<pid>/cmdline` after the process has exited.
//!
//! So these cases drive the real `ExoProcessTransport::exchange` against a child that reports its
//! own `argv`, and pin two things the existing suite does not: that the request arrives on stdin
//! and nowhere else, and that the harness adds nothing of its own to the child's stdout.

use sts2_harness::{ExoProcessConfig, ExoProcessTransport, ExoTransport};

/// A marker that cannot occur in a path, an argument, or this source file by accident.
///
/// It is deliberately not a word: an assertion matching "the request" by substring would also pass
/// if the harness echoed a field name or truncated the body, which is not what is being pinned.
const MARKER: &str = "7f3c1a9e-PRIVATE-OBSERVATION-MARKER-4b8d2f6a";

/// A request shaped like a real one: the marker inside an observation, well past a token length,
/// with the field names a prompt would actually carry.
fn marked_request() -> Vec<u8> {
    format!(
        r#"{{"schema":"sts2.exo-decision-v1","observation":{{"note":"{MARKER}","player":{{"hp":10}}}},"objective":"survive"}}"#
    )
    .into_bytes()
}

/// Run one exchange against a child that echoes back its own argv and the bytes it was given.
///
/// `/proc/self/cmdline` is NUL-separated and ends with a trailing NUL, which `tr` turns into a
/// trailing newline; that newline is the only thing between an empty argv and a present one, so
/// the report is the child's *raw* command line rather than a summary that could hide an argument.
fn exchange_reporting_argv(extra_argument: Option<&str>) -> (Vec<u8>, String) {
    // The child reports what it actually received, not what it was told, so the stdin half of the
    // report is read back from the pipe. `body=$(cat)` drains stdin exactly once; the argv half is
    // the kernel's own `/proc/self/cmdline`, which no amount of harness convention can falsify.
    let script = String::from(
        "body=$(cat); tr '\\0' '\\n' < /proc/self/cmdline; \
         printf '\\n---STDIN-EOF---\\n'; printf '%s' \"$body\"",
    );
    let mut arguments = vec![String::from("-c"), script];
    if let Some(extra) = extra_argument {
        arguments.push(String::from("sh"));
        arguments.push(String::from(extra));
    }
    let config = ExoProcessConfig::new("/bin/sh", arguments, None, Vec::new())
        .expect("fixture configuration is valid");
    let response = ExoProcessTransport::new(config)
        .exchange(&marked_request(), 65_536, 5_000)
        .expect("the reporting child answers");
    let report = String::from_utf8(response).expect("the child emits utf-8");
    let (argv, stdin) = report
        .split_once("\n---STDIN-EOF---\n")
        .expect("the child reports both channels");
    (argv.as_bytes().to_vec(), stdin.to_owned())
}

/// The request reaches the child on stdin and appears nowhere in its command line.
///
/// `/proc/self/cmdline` is the strongest available witness: it is what the kernel hands to every
/// local reader, so "absent here" is a statement about the real process, not about a harness
/// convention that a future refactor could quietly break. Refs #761.
#[cfg(unix)]
#[test]
fn the_decision_request_is_never_placed_in_the_child_command_line() {
    let (argv, stdin) = exchange_reporting_argv(None);
    let argv = String::from_utf8(argv).expect("argv is utf-8");
    assert!(
        !argv.contains(MARKER),
        "the request reached the child as an argument, so it is readable from \
         /proc/<pid>/cmdline by any local user: {argv}"
    );
    assert!(
        stdin.contains(MARKER),
        "the request never arrived on stdin, so this case is not observing the real path: {stdin}"
    );
}

/// Non-vacuity: the same capture, with the marker deliberately passed as an argument.
///
/// Without this, a child that reported nothing — or a `/proc` read that failed silently — would
/// make the case above pass for the wrong reason. This is the control that says the detector
/// works, measured through the identical code path and the identical assertion.
#[cfg(unix)]
#[test]
fn the_command_line_capture_would_notice_a_request_passed_as_an_argument() {
    let (argv, _stdin) = exchange_reporting_argv(Some(MARKER));
    let argv = String::from_utf8(argv).expect("argv is utf-8");
    assert!(
        argv.contains(MARKER),
        "the capture missed a marker that was passed as an argument, so the refusal above proves \
         nothing: {argv}"
    );
}

/// Stdout is exactly the child's own bytes, with nothing added by the harness.
///
/// The second half of the same acceptance criterion. `a_completed_exchange_reports_nothing`
/// already covers the failure line on the *harness's* stderr; this covers the positive direction,
/// where a valid exchange must not be padded, annotated or reordered on the way out. Refs #761.
#[cfg(unix)]
#[test]
fn a_successful_exchange_returns_exactly_the_child_stdout() {
    let expected = b"{\"decision\":\"wait\"}";
    let config = ExoProcessConfig::new(
        "/bin/sh",
        vec![
            String::from("-c"),
            String::from("cat >/dev/null; printf '%s' '{\"decision\":\"wait\"}'"),
        ],
        None,
        Vec::new(),
    )
    .expect("fixture configuration is valid");
    let mut transport = ExoProcessTransport::new(config);
    let response = transport
        .exchange(b"request", 512, 5_000)
        .expect("the child answers");
    assert_eq!(
        response, expected,
        "the harness altered the child's stdout; the decision payload must cross the boundary \
         byte-for-byte"
    );
    transport.close().expect("the transport closes");
}
