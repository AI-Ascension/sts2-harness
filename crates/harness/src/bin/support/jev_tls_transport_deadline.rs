// SPDX-License-Identifier: MIT

//! One absolute budget for the private, one-shot System One bridge exchange.

use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub(super) const EXPIRED: &str = "the provider exchange exceeded its deadline";

/// An expired budget is a refusal, never a fresh one-millisecond socket allowance.
pub(super) fn remaining(started: Instant) -> io::Result<Duration> {
    super::deadline()
        .checked_sub(started.elapsed())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, EXPIRED))
}

/// Guards resolution as well as socket I/O and parsing with the same deadline.
///
/// This module is private to `sts2-jev-bridge`; it is not an embedded client. The platform's
/// blocking resolver has no cancellation API. If it does not return, the owned watchdog exits
/// this bridge process with status 2, without printing a decision or blocking on an output pipe.
/// This is a refusal, not evidence that an already-sent provider request was rolled back.
pub(super) fn within<T>(
    started: Instant,
    operation: impl FnOnce() -> Result<T, Box<dyn std::error::Error>>,
) -> Result<T, Box<dyn std::error::Error>> {
    remaining(started)?;
    let mut watchdog = Watchdog::arm(started)?;
    let result = operation();
    watchdog.cancel_and_join()?;
    remaining(started)?;
    result
}

struct Watchdog {
    stop: Option<SyncSender<()>>,
    task: Option<JoinHandle<()>>,
}

impl Watchdog {
    fn arm(started: Instant) -> Result<Self, Box<dyn std::error::Error>> {
        let (stop, receiver) = mpsc::sync_channel(1);
        let task = thread::Builder::new()
            .name(String::from("jev-exchange-deadline"))
            .spawn(move || {
                let expired = match remaining(started) {
                    Ok(allowance) => matches!(
                        receiver.recv_timeout(allowance),
                        Err(RecvTimeoutError::Timeout)
                    ),
                    Err(_) => receiver.try_recv().is_err(),
                };
                if expired {
                    // Only this bridge process is terminated. No resolver thread, descendant,
                    // transport executable or external provider cancellation is implied.
                    std::process::exit(2);
                }
            })
            .map_err(|_| "the provider deadline watchdog could not be started")?;
        Ok(Self {
            stop: Some(stop),
            task: Some(task),
        })
    }

    fn cancel_and_join(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(task) = self.task.take() {
            task.join()
                .map_err(|_| "the provider deadline watchdog could not be joined")?;
        }
        Ok(())
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        let _ = self.cancel_and_join();
    }
}

/// Enforces the budget beneath rustls, including repeated I/O inside one TLS operation.
pub(crate) struct DeadlineSocket {
    stream: TcpStream,
    started: Instant,
}

impl DeadlineSocket {
    pub(crate) fn new(stream: TcpStream, started: Instant) -> Self {
        Self { stream, started }
    }

    pub(crate) fn check(&self) -> io::Result<()> {
        remaining(self.started).map(|_| ())
    }

    fn bound(&self) -> io::Result<()> {
        let allowance = remaining(self.started)?;
        self.stream
            .set_read_timeout(Some(allowance))
            .and_then(|()| self.stream.set_write_timeout(Some(allowance)))
            .map_err(|_| io::Error::other("the provider connection timeout could not be set"))
    }
}

impl Read for DeadlineSocket {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.bound()?;
        let result = self.stream.read(bytes);
        self.check()?;
        result
    }
}

impl Write for DeadlineSocket {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bound()?;
        let result = self.stream.write(bytes);
        self.check()?;
        result
    }

    fn flush(&mut self) -> io::Result<()> {
        self.bound()?;
        let result = self.stream.flush();
        self.check()?;
        result
    }
}

#[cfg(test)]
#[path = "jev_tls_transport_deadline_tests.rs"]
mod tests;
