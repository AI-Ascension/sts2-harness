// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use sts2_harness::{
    ExoProcessConfig, ExoProcessConfigError, ExoProcessTransport, ExoTransport, ExoTransportError,
};

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
