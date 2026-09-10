// SPDX-License-Identifier: MIT

use super::ollama_test_support::consume_request;
use super::*;

fn oracle_request() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "model_execution_id":"oracle-lifecycle",
        "legal_action_ids":["combat.end-turn"],
        "observation":{"state_id":"oracle-combat","generation":0},
    }))
    .expect("request")
}
#[test]
fn only_catalog_actions_and_bounded_rationale_are_accepted() {
    let ids = vec![json!("play:1")];
    assert!(validate_decision(r#"{"action_id":"play:1","rationale":"Attack"}"#, &ids).is_ok());
    assert!(validate_decision(r#"{"action_id":"invented","rationale":"Attack"}"#, &ids).is_err());
    assert!(validate_decision(r#"{"action_id":"play:1","rationale":"","extra":1}"#, &ids).is_err());
}

#[test]
fn response_failure_after_body_write_is_recorded_as_completed() -> Result<(), String> {
    use std::net::TcpListener;
    use std::thread;

    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|_| "bind".to_owned())?;
    let address = listener.local_addr().map_err(|_| "address".to_owned())?;
    let server = thread::spawn(move || -> Result<(), String> {
        let (mut stream, _) = listener.accept().map_err(|_| "accept".to_owned())?;
        consume_request(&mut stream)?;
        stream
            .write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .map_err(|_| "response".to_owned())
    });
    let mut capture =
        sts2_harness::MemoryCapture::new(sts2_harness::CaptureMode::Metadata, 4, LIMIT)
            .map_err(|_| "capture")?;
    assert!(run_with_capture_bytes(&oracle_request(), &mut capture, address).is_err());
    server.join().map_err(|_| "server".to_owned())??;
    let states = capture
        .records()
        .map(|record| record.state)
        .collect::<Vec<_>>();
    assert_eq!(
        states,
        vec![
            sts2_harness::TransportState::Prepared,
            sts2_harness::TransportState::WriteCompleted,
        ]
    );
    Ok(())
}

#[test]
fn malformed_response_after_body_write_is_recorded_as_completed() -> Result<(), String> {
    use std::net::TcpListener;
    use std::thread;

    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|_| "bind".to_owned())?;
    let address = listener.local_addr().map_err(|_| "address".to_owned())?;
    let server = thread::spawn(move || -> Result<(), String> {
        let (mut stream, _) = listener.accept().map_err(|_| "accept".to_owned())?;
        consume_request(&mut stream)?;
        stream
            .write_all(b"malformed response")
            .map_err(|_| "response".to_owned())
    });
    let mut capture =
        sts2_harness::MemoryCapture::new(sts2_harness::CaptureMode::Metadata, 4, LIMIT)
            .map_err(|_| "capture")?;
    assert!(run_with_capture_bytes(&oracle_request(), &mut capture, address).is_err());
    server.join().map_err(|_| "server".to_owned())??;
    assert_eq!(
        capture
            .records()
            .map(|record| record.state)
            .collect::<Vec<_>>(),
        vec![
            sts2_harness::TransportState::Prepared,
            sts2_harness::TransportState::WriteCompleted,
        ]
    );
    Ok(())
}

#[test]
fn response_timeout_after_body_write_is_recorded_as_completed() -> Result<(), String> {
    use std::net::TcpListener;
    use std::thread;

    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|_| "bind".to_owned())?;
    let address = listener.local_addr().map_err(|_| "address".to_owned())?;
    let server = thread::spawn(move || -> Result<(), String> {
        let (mut stream, _) = listener.accept().map_err(|_| "accept".to_owned())?;
        consume_request(&mut stream)?;
        thread::sleep(Duration::from_millis(100));
        Ok(())
    });
    let mut capture =
        sts2_harness::MemoryCapture::new(sts2_harness::CaptureMode::Metadata, 4, LIMIT)
            .map_err(|_| "capture")?;
    assert!(
        run_with_capture_bytes_timeout(
            &oracle_request(),
            &mut capture,
            address,
            Duration::from_millis(10),
        )
        .is_err()
    );
    server.join().map_err(|_| "server".to_owned())??;
    assert_eq!(
        capture
            .records()
            .map(|record| record.state)
            .collect::<Vec<_>>(),
        vec![
            sts2_harness::TransportState::Prepared,
            sts2_harness::TransportState::WriteCompleted,
        ]
    );
    Ok(())
}
