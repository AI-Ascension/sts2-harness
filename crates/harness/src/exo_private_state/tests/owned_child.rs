// SPDX-License-Identifier: MIT
#![allow(clippy::expect_used)]

use std::io::{self, Read, Write};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

const OWNED_CHILD_REAP_GRACE: Duration = Duration::from_secs(1);
const CHILD_OUTPUT_LIMIT_BYTES: u64 = 1024 * 1024;

pub(super) struct OwnedChild {
    child: Option<Child>,
    stdout: Option<ChildOutput>,
    stderr: Option<ChildOutput>,
    timeout_cleanup_attempted: bool,
}

struct ChildOutput {
    receiver: Receiver<io::Result<Vec<u8>>>,
}

#[derive(Debug)]
pub(super) struct CapturedChildOutput {
    pub(super) status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl OwnedChild {
    pub(super) fn spawn(mut command: Command) -> io::Result<Self> {
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdout = match child.stdout.take() {
            Some(stdout) => match capture_child_output(stdout) {
                Ok(output) => output,
                Err(error) => return Err(cleanup_after_capture_error(&mut child, error)),
            },
            None => {
                return Err(cleanup_after_capture_error(
                    &mut child,
                    io::Error::other("owned child stdout was not piped"),
                ));
            }
        };
        let stderr = match child.stderr.take() {
            Some(stderr) => match capture_child_output(stderr) {
                Ok(output) => output,
                Err(error) => return Err(cleanup_after_capture_error(&mut child, error)),
            },
            None => {
                return Err(cleanup_after_capture_error(
                    &mut child,
                    io::Error::other("owned child stderr was not piped"),
                ));
            }
        };
        Ok(Self {
            child: Some(child),
            stdout: Some(stdout),
            stderr: Some(stderr),
            timeout_cleanup_attempted: false,
        })
    }

    pub(super) fn child_mut(&mut self) -> &mut Child {
        self.child.as_mut().expect("owned child is still tracked")
    }

    pub(super) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.child_mut().try_wait()
    }

    pub(super) fn wait_until(&mut self, timeout: Duration) -> io::Result<CapturedChildOutput> {
        let deadline = Instant::now() + timeout;
        let reap_grace = OWNED_CHILD_REAP_GRACE.min(timeout.mul_f64(0.2));
        let execution_deadline = deadline.checked_sub(reap_grace).unwrap_or(deadline);
        loop {
            match self.try_wait() {
                Ok(Some(status)) => return self.collect(status, deadline),
                Ok(None) => {}
                Err(error) => {
                    return Err(io::Error::other(format!(
                        "owned child exit observation failed: {error}"
                    )));
                }
            }
            let now = Instant::now();
            if now >= execution_deadline {
                return self.terminate_before_deadline(deadline);
            }
            thread::sleep(Duration::from_millis(10).min(execution_deadline - now));
        }
    }

    fn terminate_before_deadline(&mut self, deadline: Instant) -> io::Result<CapturedChildOutput> {
        self.timeout_cleanup_attempted = true;
        let kill_error = self.child_mut().kill().err();
        loop {
            match self.try_wait() {
                Ok(Some(status)) => {
                    let detail = match self.collect(status, deadline) {
                        Ok(output) => format!(
                            "stdout:\n{}\nstderr:\n{}",
                            String::from_utf8_lossy(&output.stdout),
                            String::from_utf8_lossy(&output.stderr)
                        ),
                        Err(error) => format!("output collection failed: {error}"),
                    };
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        format!(
                            "owned child exceeded its deadline; child exit/reap was observed during timeout cleanup (status {status}); {}\n{detail}",
                            kill_error
                                .as_ref()
                                .map(ToString::to_string)
                                .unwrap_or_else(|| "kill request succeeded".to_owned())
                        ),
                    ));
                }
                Ok(None) => {}
                Err(error) => {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        format!(
                            "owned child exceeded its deadline; termination was requested but exit/reap could not be observed: {error}; kill result: {}",
                            kill_error
                                .as_ref()
                                .map(ToString::to_string)
                                .unwrap_or_else(|| "success".to_owned())
                        ),
                    ));
                }
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!(
                        "owned child exceeded its deadline; termination was requested but exit/reap was not observed; kill result: {}",
                        kill_error
                            .as_ref()
                            .map(ToString::to_string)
                            .unwrap_or_else(|| "success".to_owned())
                    ),
                ));
            }
            thread::sleep(Duration::from_millis(10).min(deadline - now));
        }
    }

    pub(super) fn collect(
        &mut self,
        status: ExitStatus,
        deadline: Instant,
    ) -> io::Result<CapturedChildOutput> {
        self.child.take();
        Ok(CapturedChildOutput {
            status,
            stdout: collect_child_output(&mut self.stdout, "stdout", deadline)?,
            stderr: collect_child_output(&mut self.stderr, "stderr", deadline)?,
        })
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            if self.timeout_cleanup_attempted {
                report_cleanup_error(
                    "owned child was not reaped by its deadline; no additional wait was started",
                );
            } else if let Err(error) =
                terminate_and_reap_bounded(&mut child, OWNED_CHILD_REAP_GRACE)
            {
                report_cleanup_error(&format!(
                    "owned child cleanup did not observe exit/reap: {error}"
                ));
            }
        }
        self.stdout.take();
        self.stderr.take();
    }
}

pub(super) fn require_one_passing_test(output: &CapturedChildOutput) -> io::Result<()> {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if output.status.success()
        && stdout.contains("running 1 test")
        && stdout.contains("test result: ok. 1 passed; 0 failed;")
    {
        return Ok(());
    }
    Err(io::Error::other(format!(
        "owned test helper did not run and pass exactly one test (status {}):\n{stdout}{stderr}",
        output.status
    )))
}

fn capture_child_output(reader: impl Read + Send + 'static) -> io::Result<ChildOutput> {
    let (sender, receiver) = mpsc::sync_channel(1);
    let reader_thread = thread::Builder::new()
        .name("owned-child-output".to_owned())
        .spawn(move || {
            let mut reader = reader;
            let result = read_child_output(&mut reader);
            let _ = sender.send(result);
        })?;
    drop(reader_thread);
    Ok(ChildOutput { receiver })
}

fn read_child_output(reader: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut output = Vec::new();
    reader
        .take(CHILD_OUTPUT_LIMIT_BYTES + 1)
        .read_to_end(&mut output)?;
    if output.len() as u64 > CHILD_OUTPUT_LIMIT_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "owned child output exceeded the capture limit",
        ));
    }
    Ok(output)
}

fn collect_child_output(
    reader: &mut Option<ChildOutput>,
    name: &str,
    deadline: Instant,
) -> io::Result<Vec<u8>> {
    let reader = reader
        .take()
        .ok_or_else(|| io::Error::other(format!("owned child {name} was already drained")))?;
    match reader
        .receiver
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
    {
        Ok(result) => result,
        Err(RecvTimeoutError::Timeout) => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!("owned child {name} output did not close before the deadline"),
        )),
        Err(RecvTimeoutError::Disconnected) => Err(io::Error::other(format!(
            "owned child {name} output reader stopped without a result"
        ))),
    }
}

fn cleanup_after_capture_error(child: &mut Child, error: io::Error) -> io::Error {
    match terminate_and_reap_bounded(child, OWNED_CHILD_REAP_GRACE) {
        Ok(()) => error,
        Err(cleanup) => {
            report_cleanup_error(&format!(
                "owned child capture setup failed and child exit/reap was not observed: {cleanup}"
            ));
            error
        }
    }
}

fn terminate_and_reap_bounded(child: &mut Child, timeout: Duration) -> io::Result<()> {
    if child.try_wait()?.is_some() {
        return Ok(());
    }
    let kill_error = child.kill().err();
    let deadline = Instant::now() + timeout;
    loop {
        if child.try_wait()?.is_some() {
            return Ok(());
        }
        let now = Instant::now();
        if now >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "kill was requested but exit/reap was not observed: {}",
                    kill_error
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_else(|| "kill request succeeded".to_owned())
                ),
            ));
        }
        thread::sleep(Duration::from_millis(10).min(deadline - now));
    }
}

fn report_cleanup_error(message: &str) {
    let _ = writeln!(
        io::stderr().lock(),
        "owned test child cleanup incomplete: {message}"
    );
}

#[cfg(test)]
#[path = "owned_child_tests.rs"]
mod tests;
