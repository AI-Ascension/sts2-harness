// SPDX-License-Identifier: MIT
#![forbid(unsafe_code)]

use super::test_peer::spawn_peer;
use super::test_support::Fixture;
use crate::{Deadline, TransportError, WorkerListener};

#[test]
fn native_accept_timeout_rearms_exclusive_endpoint() -> Result<(), TransportError> {
    let fixture = Fixture::new()?;
    let endpoint_nonce = super::test_support::nonce(63);
    let mut listener = WorkerListener::bind(fixture.policy_for_current(endpoint_nonce)?)?;
    for _ in 0..2 {
        assert!(matches!(
            listener.accept_authenticated(Deadline::new(Duration::from_millis(50))?),
            Err(TransportError::Deadline)
        ));
        assert!(matches!(
            WorkerListener::bind(fixture.policy_for_current(endpoint_nonce)?),
            Err(TransportError::Os)
        ));
    }
    listener.shutdown()?;
    drop(listener);
    let mut replacement = WorkerListener::bind(fixture.policy_for_current(endpoint_nonce)?)?;
    replacement.shutdown()
}

#[test]
fn native_unread_maximum_response_is_deadline_bounded() -> Result<(), TransportError> {
    let fixture = Fixture::new()?;
    let endpoint_nonce = super::test_support::nonce(62);
    let mut child = spawn_peer("unread-response", endpoint_nonce, &fixture.credential)?;
    let policy = fixture.policy_for_child(&child, endpoint_nonce)?;
    let mut listener = WorkerListener::bind(policy)?;
    let mut connection =
        listener.accept_authenticated(Deadline::new(Duration::from_millis(500))?)?;
    assert_eq!(connection.read_request()?, b"{}");
    let body = vec![b'x'; crate::MAX_FRAME_BYTES];
    assert!(matches!(
        connection.write_response(&body),
        Err(TransportError::Deadline)
    ));
    assert!(child.try_wait().map_err(|_| TransportError::Os)?.is_none());
    assert!(child.wait().map_err(|_| TransportError::Os)?.success());
    listener.shutdown()
}
use std::time::Duration;

#[test]
fn native_response_delivery_and_client_close_are_bounded() -> Result<(), TransportError> {
    for (index, mode) in [
        "delayed-reader",
        "hold-after-response",
        "extra-after-response",
    ]
    .into_iter()
    .enumerate()
    {
        let fixture = Fixture::new()?;
        let endpoint_nonce = super::test_support::nonce(50 + index as u64);
        let mut child = spawn_peer(mode, endpoint_nonce, &fixture.credential)?;
        let policy = fixture.policy_for_child(&child, endpoint_nonce)?;
        let mut listener = WorkerListener::bind(policy)?;
        let mut connection =
            listener.accept_authenticated(Deadline::new(Duration::from_millis(500))?)?;
        assert_eq!(connection.read_request()?, b"{}");
        let result = connection.write_response(b"{}");
        if mode == "delayed-reader" {
            assert!(result.is_ok());
        } else if mode == "hold-after-response" {
            assert!(matches!(result, Err(TransportError::Deadline)));
            assert!(child.try_wait().map_err(|_| TransportError::Os)?.is_none());
        } else {
            assert!(matches!(result, Err(TransportError::Framing)));
            assert!(matches!(
                connection.read_request(),
                Err(TransportError::Closed)
            ));
            assert!(matches!(
                connection.write_response(b"{}"),
                Err(TransportError::Closed)
            ));
        }
        assert!(child.wait().map_err(|_| TransportError::Os)?.success());
        listener.shutdown()?;
    }
    Ok(())
}
