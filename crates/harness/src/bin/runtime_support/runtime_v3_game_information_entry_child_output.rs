// SPDX-License-Identifier: MIT

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use std::io::{self, Read};
use std::os::fd::AsFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const MAX_CAPTURE_BYTES_PER_STREAM: usize = 256 * 1024;
const POLL_QUANTUM: Duration = Duration::from_millis(25);
const PUMP_STOP_BUDGET: Duration = Duration::from_millis(500);
const READ_BUFFER_BYTES: usize = 16 * 1024;

#[derive(Default)]
struct CapturedOutput {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    stdout_eof: bool,
    stderr_eof: bool,
    stdout_truncated: bool,
    stderr_truncated: bool,
    error: Option<String>,
    polls: u32,
}

pub(super) struct OutputPump {
    captured: Arc<Mutex<CapturedOutput>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl OutputPump {
    pub(super) fn new<Stdout, Stderr>(stdout: Stdout, stderr: Stderr, pid: u32) -> io::Result<Self>
    where
        Stdout: AsFd + Read + Send + 'static,
        Stderr: AsFd + Read + Send + 'static,
    {
        let captured = Arc::new(Mutex::new(CapturedOutput::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let worker_capture = Arc::clone(&captured);
        let worker_stop = Arc::clone(&stop);
        let worker = thread::Builder::new()
            .name(format!("runtime-child-output-{pid}"))
            .spawn(move || pump_output(stdout, stderr, worker_capture, worker_stop))?;
        Ok(Self {
            captured,
            stop,
            worker: Some(worker),
        })
    }

    pub(super) fn complete(&self) -> bool {
        let captured = lock_capture(&self.captured);
        captured.stdout_eof && captured.stderr_eof
    }

    pub(super) fn truncated(&self) -> bool {
        let captured = lock_capture(&self.captured);
        captured.stdout_truncated || captured.stderr_truncated
    }

    pub(super) fn error(&self) -> Option<String> {
        lock_capture(&self.captured).error.clone()
    }

    pub(super) fn stdout(&self) -> Vec<u8> {
        lock_capture(&self.captured).stdout.clone()
    }

    pub(super) fn stderr(&self) -> Vec<u8> {
        lock_capture(&self.captured).stderr.clone()
    }

    pub(super) fn worker_stopped(&self) -> bool {
        self.worker.is_none()
    }

    fn poll_count(&self) -> u32 {
        lock_capture(&self.captured).polls
    }

    pub(super) fn status_summary(&self) -> String {
        let captured = lock_capture(&self.captured);
        format!(
            "stdout_eof={}, stderr_eof={}, stdout_truncated={}, stderr_truncated={}, output_error={}",
            captured.stdout_eof,
            captured.stderr_eof,
            captured.stdout_truncated,
            captured.stderr_truncated,
            captured.error.as_deref().unwrap_or("none"),
        )
    }

    pub(super) fn take(&mut self) -> io::Result<Option<(Vec<u8>, Vec<u8>)>> {
        self.stop()?;
        let mut captured = lock_capture(&self.captured);
        if !captured.stdout_eof || !captured.stderr_eof {
            return Ok(None);
        }
        Ok(Some((
            std::mem::take(&mut captured.stdout),
            std::mem::take(&mut captured.stderr),
        )))
    }

    pub(super) fn stop(&mut self) -> io::Result<()> {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let deadline = Instant::now() + PUMP_STOP_BUDGET;
            while !worker.is_finished() && Instant::now() < deadline {
                thread::sleep(POLL_QUANTUM.min(deadline.saturating_duration_since(Instant::now())));
            }
            if !worker.is_finished() {
                let message = "runtime child output pump exceeded its bounded stop deadline";
                lock_capture(&self.captured).error = Some(message.to_string());
                return Err(io::Error::new(io::ErrorKind::TimedOut, message));
            }
            if worker.join().is_err() {
                let message = "runtime child output pump panicked";
                lock_capture(&self.captured).error = Some(message.to_string());
                return Err(io::Error::other(message));
            }
        }
        Ok(())
    }
}

impl Drop for OutputPump {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn pump_output(
    stdout: impl AsFd + Read,
    stderr: impl AsFd + Read,
    captured: Arc<Mutex<CapturedOutput>>,
    stop: Arc<AtomicBool>,
) {
    let mut stdout = Some(stdout);
    let mut stderr = Some(stderr);
    let timeout = Timespec::try_from(POLL_QUANTUM).expect("short output-poll interval");
    loop {
        if stop.load(Ordering::Acquire) || (stdout.is_none() && stderr.is_none()) {
            return;
        }
        let (stdout_ready, stderr_ready) = {
            let mut descriptors = Vec::with_capacity(2);
            let stdout_index = stdout.as_ref().map(|pipe| {
                let index = descriptors.len();
                descriptors.push(PollFd::new(
                    pipe,
                    PollFlags::IN | PollFlags::HUP | PollFlags::ERR,
                ));
                index
            });
            let stderr_index = stderr.as_ref().map(|pipe| {
                let index = descriptors.len();
                descriptors.push(PollFd::new(
                    pipe,
                    PollFlags::IN | PollFlags::HUP | PollFlags::ERR,
                ));
                index
            });
            match poll(&mut descriptors, Some(&timeout)) {
                Ok(_) => {
                    let mut captured = lock_capture(&captured);
                    captured.polls = captured.polls.saturating_add(1);
                    drop(captured);
                    let ready = |index: Option<usize>| {
                        index.is_some_and(|index| {
                            let events = descriptors[index].revents();
                            events.intersects(PollFlags::NVAL)
                                || events
                                    .intersects(PollFlags::IN | PollFlags::HUP | PollFlags::ERR)
                        })
                    };
                    (ready(stdout_index), ready(stderr_index))
                }
                Err(error) if error == rustix::io::Errno::INTR => continue,
                Err(error) => {
                    lock_capture(&captured).error = Some(error.to_string());
                    return;
                }
            }
        };

        // Poll covers both pipes; a single bounded read per ready pipe preserves fairness.
        if stdout_ready
            && let Some(pipe) = stdout.as_mut()
            && read_output(pipe, true, &captured)
        {
            stdout = None;
        }
        if stderr_ready
            && let Some(pipe) = stderr.as_mut()
            && read_output(pipe, false, &captured)
        {
            stderr = None;
        }
    }
}

fn read_output<R: Read>(pipe: &mut R, stdout: bool, captured: &Mutex<CapturedOutput>) -> bool {
    let mut buffer = [0_u8; READ_BUFFER_BYTES];
    match pipe.read(&mut buffer) {
        Ok(0) => {
            let mut captured = lock_capture(captured);
            if stdout {
                captured.stdout_eof = true;
            } else {
                captured.stderr_eof = true;
            }
            true
        }
        Ok(read) => {
            let mut captured = lock_capture(captured);
            let CapturedOutput {
                stdout: stdout_output,
                stderr: stderr_output,
                stdout_truncated,
                stderr_truncated,
                ..
            } = &mut *captured;
            let (output, truncated) = if stdout {
                (stdout_output, stdout_truncated)
            } else {
                (stderr_output, stderr_truncated)
            };
            let available = MAX_CAPTURE_BYTES_PER_STREAM.saturating_sub(output.len());
            let retained = read.min(available);
            output.extend_from_slice(&buffer[..retained]);
            *truncated |= retained < read;
            false
        }
        Err(error) if error.kind() == io::ErrorKind::Interrupted => false,
        Err(error) => {
            lock_capture(captured).error = Some(error.to_string());
            true
        }
    }
}

fn lock_capture(captured: &Mutex<CapturedOutput>) -> MutexGuard<'_, CapturedOutput> {
    captured
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
#[path = "runtime_v3_game_information_entry_child_output_tests.rs"]
mod tests;
