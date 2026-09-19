// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use sts2_harness::{ExoProcessConfig, ExoProcessConfigError};
#[cfg(unix)]
use sts2_harness::{ExoProcessTransport, ExoTransport, ExoTransportError};

#[test]
fn process_configuration_requires_direct_bounded_inputs() {
    assert_eq!(
        ExoProcessConfig::new("", Vec::new(), None, Vec::new()),
        Err(ExoProcessConfigError::Invalid)
    );
    assert_eq!(
        ExoProcessConfig::new(
            "/bin/bridge",
            vec![String::from("--ok")],
            None,
            vec![String::from("LD_PRELOAD")]
        ),
        Err(ExoProcessConfigError::Invalid)
    );
    assert_eq!(
        ExoProcessConfig::new(
            "/bin/bridge",
            vec![String::from("--ok")],
            None,
            vec![String::from("EXO_API_KEY"), String::from("EXO_API_KEY")]
        ),
        Err(ExoProcessConfigError::Invalid)
    );
}

#[cfg(unix)]
#[test]
fn process_transport_passes_one_request_on_stdin_and_bounds_response() {
    let script = String::from(
        "cat >/dev/null; printf '%s' '{\"decision\":\"reobserve\",\"rationale\":\"refresh\"}'",
    );
    let config = ExoProcessConfig::new(
        "/bin/sh",
        vec![String::from("-c"), script],
        None,
        Vec::new(),
    )
    .expect("shell bridge configuration is valid");
    let mut transport = ExoProcessTransport::new(config);
    let response = transport
        .exchange(b"sanitized request", 512, 2_000)
        .expect("bridge response is returned");
    assert_eq!(
        response,
        br#"{"decision":"reobserve","rationale":"refresh"}"#.to_vec()
    );
}

#[cfg(unix)]
#[test]
fn process_transport_times_out_and_rejects_oversized_output() {
    let timeout_config = ExoProcessConfig::new(
        "/bin/sh",
        vec![
            String::from("-c"),
            String::from("cat >/dev/null; sleep 1; printf '%s' '{}'"),
        ],
        None,
        Vec::new(),
    )
    .expect("timeout bridge configuration is valid");
    let mut timeout_transport = ExoProcessTransport::new(timeout_config);
    assert_eq!(
        timeout_transport.exchange(b"request", 512, 20),
        Err(ExoTransportError::Timeout)
    );

    let oversized_config = ExoProcessConfig::new(
        "/bin/sh",
        vec![
            String::from("-c"),
            String::from("cat >/dev/null; printf '0123456789'"),
        ],
        None,
        Vec::new(),
    )
    .expect("oversized bridge configuration is valid");
    let mut oversized_transport = ExoProcessTransport::new(oversized_config);
    assert_eq!(
        oversized_transport.exchange(b"request", 4, 2_000),
        Err(ExoTransportError::OversizedResponse)
    );
}

#[cfg(unix)]
#[test]
fn process_transport_close_is_fail_closed() {
    let config = ExoProcessConfig::new("/bin/printf", vec![String::from("{}")], None, Vec::new())
        .expect("bridge configuration is valid");
    let mut transport = ExoProcessTransport::new(config);
    transport.close().expect("close succeeds");
    assert_eq!(
        transport.exchange(b"request", 512, 2_000),
        Err(ExoTransportError::MalformedResponse)
    );
}

#[cfg(unix)]
#[test]
fn deadline_includes_a_request_larger_than_an_unread_stdin_pipe() {
    let config = ExoProcessConfig::new("/bin/sleep", vec![String::from("2")], None, Vec::new())
        .expect("sleep fixture configuration");
    let mut transport = ExoProcessTransport::new(config);
    let started = std::time::Instant::now();
    assert_eq!(
        transport.exchange(&vec![b'x'; 1024 * 1024], 512, 20),
        Err(ExoTransportError::Timeout)
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
}

#[cfg(unix)]
#[test]
fn drains_stdout_while_writing_a_large_request() {
    let config = ExoProcessConfig::new(
        "/bin/sh",
        vec![
            String::from("-c"),
            String::from("head -c 1048576 /dev/zero; cat >/dev/null"),
        ],
        None,
        Vec::new(),
    )
    .expect("duplex fixture configuration");
    let mut transport = ExoProcessTransport::new(config);
    assert_eq!(
        transport
            .exchange(&vec![b'x'; 1024 * 1024], 1024 * 1024, 2_000)
            .expect("both pipes make progress"),
        vec![0; 1024 * 1024]
    );
}

#[cfg(unix)]
#[test]
fn synchronous_transport_can_be_called_inside_an_async_runtime() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("outer runtime");
    runtime.block_on(async {
        let config = ExoProcessConfig::new(
            "/bin/sh",
            vec![
                String::from("-c"),
                String::from("cat >/dev/null; printf '{}'"),
            ],
            None,
            Vec::new(),
        )
        .expect("fixture configuration");
        assert_eq!(
            ExoProcessTransport::new(config).exchange(b"request", 512, 2000),
            Ok(b"{}".to_vec())
        );
    });
}

/// Raise one failed exchange in a process of its own.
///
/// The child's standard error is the only channel that names a transport that cannot start, and the
/// harness now reports it on its own standard error, which a test can only read back from another
/// process. A run without the case name is a no-op, so the ordinary test run stays unaffected.
#[cfg(unix)]
#[test]
fn provider_transport_failure_child_helper() {
    let Some(case) = std::env::var_os("STS2_TEST_EXO_FAILURE_CASE") else {
        return;
    };
    let script = match case.to_str().expect("case name") {
        "success" => String::from("cat >/dev/null; printf '%s' '{}'"),
        "message" => String::from("printf '%s\\n' 'powershell.exe is not recognized' >&2; exit 7"),
        _ => String::from(
            "head -c 2048 /dev/zero | tr '\\0' n >&2; printf 'marker\\033' >&2; head -c 2041 /dev/zero | tr '\\0' n >&2; exit 3",
        ),
    };
    let config = ExoProcessConfig::new(
        "/bin/sh",
        vec![String::from("-c"), script],
        None,
        Vec::new(),
    )
    .expect("bridge configuration is valid");
    let result = ExoProcessTransport::new(config).exchange(b"request", 512, 5_000);
    if case.to_str().expect("case name") == "success" {
        assert_eq!(result, Ok(b"{}".to_vec()));
    } else {
        assert_eq!(result, Err(ExoTransportError::Unavailable));
    }
}

/// Run the helper in a child process and return the harness's own standard error from it.
#[cfg(unix)]
fn failure_helper_stderr(case: &str) -> String {
    let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
        .args([
            "--exact",
            "provider_transport_failure_child_helper",
            "--nocapture",
        ])
        .env("STS2_TEST_EXO_FAILURE_CASE", case)
        .output()
        .expect("child test process");
    assert!(
        output.status.success(),
        "child helper failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// A transport that cannot start is reported with its exit status and its own message, because the
/// harness discarded the child's standard error and called the result an outage. Refs #348.
#[cfg(unix)]
#[test]
fn a_transport_that_cannot_start_reports_its_exit_status_and_message() {
    let stderr = failure_helper_stderr("message");
    assert!(
        stderr.contains("provider transport failed: exit status: 7"),
        "no operator line naming the exit status: {stderr}"
    );
    assert!(
        stderr.contains("powershell.exe is not recognized"),
        "no operator line carrying the child's message: {stderr}"
    );
}

/// A bridge that writes without limit cannot flood the operator log, and a control byte cannot move
/// the cursor or split the line: only a bounded, escaped tail of the stream is reported.
#[cfg(unix)]
#[test]
fn a_flooding_bridge_is_reported_as_a_bounded_escaped_tail() {
    let stderr = failure_helper_stderr("noisy");
    assert!(
        stderr.contains("provider transport failed: exit status: 3"),
        "no operator line naming the exit status: {stderr}"
    );
    assert!(
        stderr.contains("stderr tail: marker\\u{1b}"),
        "the report must keep the end of the stream rather than its start: {stderr}"
    );
    assert!(!stderr.contains('\u{1b}'), "a raw control byte was written");
    let line = stderr
        .lines()
        .find(|line| line.contains("provider transport failed"))
        .expect("one operator line");
    assert!(
        line.chars().count() < 400,
        "the operator line is unbounded: {} characters",
        line.chars().count()
    );
}

/// A completed exchange says nothing: the report exists for a transport that did not start.
#[cfg(unix)]
#[test]
fn a_completed_exchange_reports_nothing() {
    let stderr = failure_helper_stderr("success");
    assert!(
        !stderr.contains("provider transport failed"),
        "a completed exchange reported a failure: {stderr}"
    );
}

/// The discarded stream was the defect, so the guard reads the transport source: restoring the null
/// device fails here instead of silently on the next native launch.
#[test]
fn the_process_transport_never_discards_the_child_standard_error() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/exo_process.rs");
    let source = std::fs::read_to_string(&path).expect("transport source");
    assert!(
        !source.contains("Stdio::null()"),
        "the child's standard error is discarded again"
    );
    assert!(
        source.contains(".stderr(Stdio::piped())"),
        "the child's standard error is no longer captured"
    );
    assert!(
        source.contains("report_stopped_child_failure(PROVIDER_TRANSPORT, &mut child, own_exit)"),
        "a captured standard error is no longer reported"
    );
}
