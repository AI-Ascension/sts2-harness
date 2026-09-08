// SPDX-License-Identifier: MIT

//! Bounded worker transport framing. This module does not authenticate peers,
//! decode requests, or admit work. The endpoint owns those ordered boundaries.

use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::time::{Instant, timeout_at};

use crate::worker_handoff::MAX_FRAME_BYTES;

/// One process-local budget, created before connection/accept and peer checks.
/// Moving it into framing preserves the time already spent authenticating.
pub struct ConnectionDeadline(Instant);

impl ConnectionDeadline {
    pub fn start(timeout: Duration) -> Result<Self, FrameIoError> {
        if timeout.is_zero() || timeout > Duration::from_secs(5) {
            return Err(FrameIoError::InvalidLimit);
        }
        Ok(Self(Instant::now() + timeout))
    }

    /// Native adapters use this same instant for all earlier connection phases.
    pub fn instant(&self) -> Instant {
        self.0
    }
}

/// Deliberately omits OS messages, peer identities, paths and frame contents.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameIoError {
    InvalidLimit,
    InvalidLength,
    Deadline,
    Transport,
    Closed,
}

impl std::fmt::Display for FrameIoError {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        output.write_str(match self {
            Self::InvalidLimit => "invalid worker frame limit",
            Self::InvalidLength => "invalid worker frame length",
            Self::Deadline => "worker connection deadline exceeded",
            Self::Transport => "worker frame transport failed",
            Self::Closed => "worker frame connection closed",
        })
    }
}

impl std::error::Error for FrameIoError {}

/// Owns the stream; no tasks, reader threads, or per-frame timers are spawned.
/// Any failed or cancelled exchange poisons further I/O. Drop releases the stream.
pub struct WorkerFrameIo<S> {
    stream: S,
    deadline: Instant,
    available: bool,
}

impl<S: AsyncRead + AsyncWrite + Unpin> WorkerFrameIo<S> {
    pub fn new(stream: S, deadline: ConnectionDeadline) -> Self {
        Self {
            stream,
            deadline: deadline.0,
            available: true,
        }
    }

    /// A request may shorten, but can never extend, the connection budget.
    pub fn restrict_timeout(&mut self, timeout: Duration) -> Result<(), FrameIoError> {
        let requested = ConnectionDeadline::start(timeout)?;
        self.deadline = self.deadline.min(requested.0);
        Ok(())
    }

    fn begin(&mut self, limit: usize) -> Result<(), FrameIoError> {
        if !self.available {
            return Err(FrameIoError::Closed);
        }
        self.available = false;
        if limit == 0 || limit > MAX_FRAME_BYTES {
            return Err(FrameIoError::InvalidLimit);
        }
        if Instant::now() >= self.deadline {
            return Err(FrameIoError::Deadline);
        }
        Ok(())
    }

    fn finish<T>(&mut self, result: Result<T, FrameIoError>) -> Result<T, FrameIoError> {
        if Instant::now() >= self.deadline {
            return Err(FrameIoError::Deadline);
        }
        if result.is_ok() {
            self.available = true;
        }
        result
    }

    /// Check the prefix against a phase-specific bound before body allocation.
    /// Successful partial reads never reset the absolute deadline.
    pub async fn read_frame(&mut self, limit: usize) -> Result<Vec<u8>, FrameIoError> {
        self.begin(limit)?;
        let result = timeout_at(self.deadline, async {
            let mut prefix = [0; 4];
            self.stream
                .read_exact(&mut prefix)
                .await
                .map_err(|_| FrameIoError::Transport)?;
            let length = usize::try_from(u32::from_be_bytes(prefix))
                .map_err(|_| FrameIoError::InvalidLength)?;
            if length == 0 || length > limit {
                return Err(FrameIoError::InvalidLength);
            }
            let mut body = vec![0; length];
            self.stream
                .read_exact(&mut body)
                .await
                .map_err(|_| FrameIoError::Transport)?;
            Ok(body)
        })
        .await
        .map_err(|_| FrameIoError::Deadline)?;
        self.finish(result)
    }

    /// Writes one prefix/body under the original connection deadline.
    pub async fn write_frame(&mut self, body: &[u8], limit: usize) -> Result<(), FrameIoError> {
        self.begin(limit)?;
        if body.is_empty() || body.len() > limit {
            return Err(FrameIoError::InvalidLength);
        }
        let prefix = u32::try_from(body.len())
            .map_err(|_| FrameIoError::InvalidLength)?
            .to_be_bytes();
        let result = timeout_at(self.deadline, async {
            self.stream
                .write_all(&prefix)
                .await
                .map_err(|_| FrameIoError::Transport)?;
            self.stream
                .write_all(body)
                .await
                .map_err(|_| FrameIoError::Transport)?;
            self.stream
                .flush()
                .await
                .map_err(|_| FrameIoError::Transport)
        })
        .await
        .map_err(|_| FrameIoError::Deadline)?;
        self.finish(result)
    }
}
