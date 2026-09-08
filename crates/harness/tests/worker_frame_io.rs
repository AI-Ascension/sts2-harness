// SPDX-License-Identifier: MIT

use std::future::{Future, poll_fn};
use std::task::Poll;
use std::time::Duration;
use sts2_harness::worker_frame_io::{ConnectionDeadline, FrameIoError, WorkerFrameIo};
use tokio::io::{AsyncReadExt, AsyncWriteExt, duplex};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[tokio::test]
async fn exact_limit_roundtrip_with_partial_pipe_io() -> TestResult {
    let (left, right) = duplex(31);
    let mut sender = WorkerFrameIo::new(left, ConnectionDeadline::start(Duration::from_secs(5))?);
    let mut receiver =
        WorkerFrameIo::new(right, ConnectionDeadline::start(Duration::from_secs(5))?);
    let payload = vec![b'x'; 65_536];
    let (written, read) = tokio::join!(
        sender.write_frame(&payload, 65_536),
        receiver.read_frame(65_536)
    );
    written?;
    assert_eq!(read?, payload);
    Ok(())
}

#[tokio::test]
async fn invalid_prefix_fails_without_waiting_for_body_and_poisons_connection() -> TestResult {
    for length in [0_u32, 65, u32::MAX] {
        let (mut peer, stream) = duplex(8);
        peer.write_all(&length.to_be_bytes()).await?;
        let mut frames =
            WorkerFrameIo::new(stream, ConnectionDeadline::start(Duration::from_secs(1))?);
        assert_eq!(
            frames.read_frame(64).await,
            Err(FrameIoError::InvalidLength)
        );
        assert_eq!(frames.read_frame(64).await, Err(FrameIoError::Closed));
    }
    Ok(())
}

#[tokio::test]
async fn deadline_includes_time_before_framing_and_cannot_be_extended() -> TestResult {
    let deadline = ConnectionDeadline::start(Duration::from_millis(5))?;
    tokio::time::sleep_until(deadline.instant()).await;
    let (mut peer, stream) = duplex(16);
    peer.write_all(&[0, 0, 0, 1, b'x']).await?;
    let mut frames = WorkerFrameIo::new(stream, deadline);
    frames.restrict_timeout(Duration::from_secs(5))?;
    assert_eq!(frames.read_frame(16).await, Err(FrameIoError::Deadline));
    assert_eq!(
        frames.write_frame(b"x", 16).await,
        Err(FrameIoError::Closed)
    );
    Ok(())
}

#[tokio::test]
async fn cancelled_partial_read_cannot_resume_or_write() -> TestResult {
    let (mut peer, stream) = duplex(16);
    peer.write_all(&[0, 0]).await?;
    let mut frames = WorkerFrameIo::new(stream, ConnectionDeadline::start(Duration::from_secs(1))?);
    let mut pending = Box::pin(frames.read_frame(16));
    let was_pending = poll_fn(|cx| Poll::Ready(pending.as_mut().poll(cx).is_pending())).await;
    assert!(was_pending);
    drop(pending);
    assert_eq!(frames.read_frame(16).await, Err(FrameIoError::Closed));
    assert_eq!(
        frames.write_frame(b"x", 16).await,
        Err(FrameIoError::Closed)
    );
    Ok(())
}

#[tokio::test]
async fn cancelled_partial_write_cannot_restart_prefix() -> TestResult {
    let (mut peer, stream) = duplex(2);
    let mut frames = WorkerFrameIo::new(stream, ConnectionDeadline::start(Duration::from_secs(1))?);
    let mut pending = Box::pin(frames.write_frame(b"payload", 16));
    let was_pending = poll_fn(|cx| Poll::Ready(pending.as_mut().poll(cx).is_pending())).await;
    assert!(was_pending);
    drop(pending);
    let mut prefix_fragment = [0; 2];
    peer.read_exact(&mut prefix_fragment).await?;
    assert_eq!(prefix_fragment, [0, 0]);
    assert_eq!(
        frames.write_frame(b"payload", 16).await,
        Err(FrameIoError::Closed)
    );
    Ok(())
}

#[tokio::test]
async fn eof_and_invalid_outgoing_length_fail_closed() -> TestResult {
    let (peer, stream) = duplex(8);
    drop(peer);
    let mut frames = WorkerFrameIo::new(stream, ConnectionDeadline::start(Duration::from_secs(1))?);
    assert_eq!(frames.read_frame(8).await, Err(FrameIoError::Transport));
    assert_eq!(frames.read_frame(8).await, Err(FrameIoError::Closed));
    for body in [Vec::new(), vec![0; 9]] {
        let (_peer, stream) = duplex(16);
        let mut frames =
            WorkerFrameIo::new(stream, ConnectionDeadline::start(Duration::from_secs(1))?);
        assert_eq!(
            frames.write_frame(&body, 8).await,
            Err(FrameIoError::InvalidLength)
        );
        assert_eq!(frames.write_frame(b"x", 8).await, Err(FrameIoError::Closed));
    }
    Ok(())
}

#[tokio::test]
async fn slow_body_and_unread_writer_expire_with_one_budget() -> TestResult {
    let (mut peer, stream) = duplex(8);
    peer.write_all(&[0, 0, 0, 2, b'x']).await?;
    let mut frames = WorkerFrameIo::new(
        stream,
        ConnectionDeadline::start(Duration::from_millis(20))?,
    );
    assert_eq!(frames.read_frame(8).await, Err(FrameIoError::Deadline));
    let (_peer, stream) = duplex(1);
    let mut frames = WorkerFrameIo::new(
        stream,
        ConnectionDeadline::start(Duration::from_millis(20))?,
    );
    assert_eq!(
        frames.write_frame(b"xx", 8).await,
        Err(FrameIoError::Deadline)
    );
    Ok(())
}

#[test]
fn invalid_connection_budgets_are_rejected() {
    for timeout in [Duration::ZERO, Duration::from_millis(5001), Duration::MAX] {
        assert!(matches!(
            ConnectionDeadline::start(timeout),
            Err(FrameIoError::InvalidLimit)
        ));
    }
}
