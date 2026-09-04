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
