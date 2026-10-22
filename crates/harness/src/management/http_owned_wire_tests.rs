// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use super::*;
use std::time::{Duration, Instant};

fn frame(body: &[u8]) -> Vec<u8> {
    let mut bytes = (body.len() as u32).to_be_bytes().to_vec();
    bytes.extend_from_slice(body);
    bytes
}

#[test]
fn fragmented_private_frames_preserve_bytes_and_do_not_wait_for_missing_input() {
    let (socket, mut peer) = UnixStream::pair().expect("pair");
    let mut frames = Frames::new(socket).expect("frames");
    let body = br#"{"kind":"stop","generation":7}"#;
    let bytes = frame(body);
    for chunk in bytes[..bytes.len() - 1].chunks(3) {
        peer.write_all(chunk).expect("fragment");
        assert!(frames.receive::<ToBackend>().expect("partial").is_none());
    }
    peer.write_all(&bytes[bytes.len() - 1..])
        .expect("final byte");
    assert!(matches!(
        frames.receive(),
        Ok(Some(ToBackend::Stop { generation: 7 }))
    ));
    let started = Instant::now();
    assert!(frames.receive::<ToBackend>().expect("idle").is_none());
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn truncated_oversized_and_unknown_field_frames_fail_closed() {
    let (socket, mut peer) = UnixStream::pair().expect("pair");
    let mut frames = Frames::new(socket).expect("frames");
    peer.write_all(&[0, 0, 0, 20, b'{'])
        .expect("truncated frame");
    assert!(frames.receive::<ToBackend>().expect("partial").is_none());
    drop(peer);
    assert!(matches!(
        frames.receive::<ToBackend>(),
        Err(WireError::Closed)
    ));

    for bytes in [
        u32::MAX.to_be_bytes().to_vec(),
        0_u32.to_be_bytes().to_vec(),
        frame(br#"{"kind":"stop","generation":7,"credential":"not-a-field"}"#),
    ] {
        let (socket, mut peer) = UnixStream::pair().expect("pair");
        let mut frames = Frames::new(socket).expect("frames");
        peer.write_all(&bytes).expect("invalid frame");
        assert!(matches!(
            frames.receive::<ToBackend>(),
            Err(WireError::Bounds | WireError::Malformed)
        ));
    }
}

#[test]
fn private_output_backpressure_has_finite_capacity_and_preserves_the_frame() {
    let (socket, peer) = UnixStream::pair().expect("pair");
    let mut sender = Frames::new(socket).expect("sender");
    let mut receiver = Frames::new(peer).expect("receiver");
    let body = vec![255_u8; MAX_RESPONSE_BYTES];
    sender
        .queue(&FromBackend::Response {
            generation: 1,
            sequence: 2,
            status: 200,
            body: body.clone(),
        })
        .expect("bounded response");
    let started = Instant::now();
    for _ in 0..128 {
        sender.flush().expect("backpressured write is nonblocking");
    }
    assert!(
        !sender.output_empty(),
        "the peer deliberately did not consume the large frame"
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        sender.flush().expect("write progress");
        if let Some(FromBackend::Response {
            generation,
            sequence,
            status,
            body: received,
        }) = receiver.receive().expect("receive progress")
        {
            assert_eq!((generation, sequence, status), (1, 2, 200));
            assert_eq!(received, body);
            break;
        }
        assert!(
            Instant::now() < deadline,
            "bounded frame must eventually finish"
        );
    }
    assert!(sender.output_empty());
    assert_eq!(sender.output_bytes, 0);
    for _ in 0..MAX_QUEUED_FRAMES {
        sender
            .queue(&ToBackend::Stop { generation: 1 })
            .expect("admitted frame");
    }
    assert_eq!(
        sender.queue(&ToBackend::Stop { generation: 1 }),
        Err(WireError::Capacity)
    );
}

#[test]
fn raw_request_and_response_limits_are_rechecked_after_private_decode() {
    let request = Request {
        method: "GET".to_owned(),
        path: "/v1/health".to_owned(),
        query: BTreeMap::new(),
        headers: BTreeMap::new(),
        body: vec![0; MAX_JSON_BYTES + 1],
    };
    assert!(matches!(
        request.into_http(&HttpLimits::default()),
        Err(WireError::Bounds)
    ));
    assert!(matches!(
        into_response(200, vec![0; MAX_RESPONSE_BYTES + 1], &HttpLimits::default()),
        Err(WireError::Bounds)
    ));
    assert!(matches!(
        into_response(302, Vec::new(), &HttpLimits::default()),
        Err(WireError::Bounds)
    ));
}

#[test]
fn absolute_monotonic_deadlines_keep_the_original_expiry_across_wire_decoding() {
    use crate::management::http::owned_deadline::Deadline;
    let deadline = Deadline::after(Duration::from_millis(10)).expect("clock");
    std::thread::sleep(Duration::from_millis(15));
    let restored = Deadline::from_wire(deadline.wire(), Duration::from_secs(300))
        .expect("valid expired deadline");
    assert_eq!(restored.wire(), deadline.wire());
    assert!(restored.expired());
    assert!(Deadline::from_wire(0, Duration::from_secs(300)).is_none());
    assert!(Deadline::from_wire(u64::MAX, Duration::from_secs(300)).is_none());
}
