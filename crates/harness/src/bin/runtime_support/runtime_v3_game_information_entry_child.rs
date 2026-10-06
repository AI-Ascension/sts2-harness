// SPDX-License-Identifier: MIT

use super::child_output::OutputPump;
use std::io;
use std::process::{Child, Output};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const DROP_REAP_BUDGET: Duration = Duration::from_secs(1);
const KILL_REAP_BUDGET: Duration = Duration::from_secs(2);
const OUTPUT_DRAIN_BUDGET: Duration = Duration::from_millis(500);
const PROCESS_DEADLINE: Duration = Duration::from_secs(30);
const POLL_QUANTUM: Duration = Duration::from_millis(25);

pub(super) struct RuntimeChild {
    child: Child,
    output: OutputPump,
    status: Option<std::process::ExitStatus>,
    reaped: Arc<AtomicBool>,
}

impl RuntimeChild {
    pub(super) fn new(mut child: Child) -> io::Result<Self> {
        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => return Err(reap_unmonitored(&mut child, "missing child stdout pipe")),
        };
        let stderr = match child.stderr.take() {
            Some(stderr) => stderr,
            None => return Err(reap_unmonitored(&mut child, "missing child stderr pipe")),
        };
        let output = match OutputPump::new(stdout, stderr, child.id()) {
            Ok(output) => output,
            Err(error) => return Err(reap_unmonitored(&mut child, &error.to_string())),
        };
        Ok(Self {
            child,
            output,
            status: None,
            reaped: Arc::new(AtomicBool::new(false)),
        })
    }

    pub(super) fn id(&self) -> u32 {
        self.child.id()
    }

    pub(super) fn try_wait(&mut self) -> io::Result<Option<std::process::ExitStatus>> {
        if self.status.is_none() {
            self.status = self.child.try_wait()?;
        }
        if self.status.is_some() {
            self.reaped.store(true, Ordering::Release);
        }
        Ok(self.status)
    }

    pub(super) fn reaped_probe(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.reaped)
    }

    pub(super) fn kill(&mut self) -> io::Result<()> {
        self.child.kill()
    }

    pub(super) fn output_complete(&self) -> bool {
        self.output.complete()
    }

    pub(super) fn output_truncated(&self) -> bool {
        self.output.truncated()
    }

    pub(super) fn output_error(&self) -> Option<String> {
        self.output.error()
    }

    pub(super) fn captured_stdout(&self) -> Vec<u8> {
        self.output.stdout()
    }

    pub(super) fn captured_stderr(&self) -> Vec<u8> {
        self.output.stderr()
    }

    pub(super) fn take_output(&mut self) -> io::Result<Option<Output>> {
        let Some((stdout, stderr)) = self.output.take()? else {
            return Ok(None);
        };
        Ok(Some(Output {
            status: self
                .status
                .expect("reaped runtime child has an exit status"),
            stdout,
            stderr,
        }))
    }
}

pub(super) fn finish_child(mut child: RuntimeChild) -> Output {
    let process_deadline = Instant::now() + PROCESS_DEADLINE;
    let mut kill_deadline = None;
    let mut output_deadline = None;
    loop {
        if let Some(error) = child.output_error() {
            panic!("owned runtime child output pump failed: {error}");
        }
        let now = Instant::now();
        let exited = child
            .try_wait()
            .unwrap_or_else(|error| panic!("poll owned runtime process: {error}"))
            .is_some();
        if exited {
            let deadline = *output_deadline.get_or_insert(now + OUTPUT_DRAIN_BUDGET);
            if child.output_complete() {
                let output = child
                    .take_output()
                    .unwrap_or_else(|error| panic!("stop owned runtime output pump: {error}"))
                    .expect("closed output pipes after child exit");
                assert!(
                    !child.output_truncated(),
                    "runtime child output exceeded the bounded capture capacity; pid={}",
                    child.id()
                );
                if kill_deadline.is_some() {
                    panic!(
                        "actual runtime entry exceeded its bounded test deadline and was killed; pid={}, stdout:\n{}\nstderr:\n{}",
                        child.id(),
                        String::from_utf8_lossy(&output.stdout),
                        String::from_utf8_lossy(&output.stderr),
                    );
                }
                return output;
            }
            if now >= deadline {
                panic!(
                    "runtime child exited but a descendant kept an output pipe open past the bounded drain deadline; pid={}, captured stdout:\n{}\nstderr:\n{}",
                    child.id(),
                    String::from_utf8_lossy(&child.captured_stdout()),
                    String::from_utf8_lossy(&child.captured_stderr()),
                );
            }
        } else if let Some(deadline) = kill_deadline {
            if now >= deadline {
                panic!(
                    "owned runtime child could not be reaped within the bounded post-kill deadline; pid={}, stdout:\n{}\nstderr:\n{}",
                    child.id(),
                    String::from_utf8_lossy(&child.captured_stdout()),
                    String::from_utf8_lossy(&child.captured_stderr()),
                );
            }
        } else if now >= process_deadline {
            kill_deadline = Some(now + KILL_REAP_BUDGET);
            let _ = child.kill();
        }

        let next_deadline = if exited {
            output_deadline.expect("output drain deadline after child exit")
        } else {
            kill_deadline.unwrap_or(process_deadline)
        };
        let wait = POLL_QUANTUM.min(next_deadline.saturating_duration_since(Instant::now()));
        if !wait.is_zero() {
            thread::sleep(wait);
        }
    }
}

fn reap_until(child: &mut Child, deadline: Instant) -> bool {
    while Instant::now() < deadline {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return true;
        }
        thread::sleep(POLL_QUANTUM.min(deadline.saturating_duration_since(Instant::now())));
    }
    matches!(child.try_wait(), Ok(Some(_)))
}

fn reap_unmonitored(child: &mut Child, reason: &str) -> io::Error {
    let pid = child.id();
    let _ = child.kill();
    if !reap_until(child, Instant::now() + DROP_REAP_BUDGET) {
        eprintln!(
            "runtime test child cleanup incomplete: pid={pid}, reaped=false, setup_error={reason}"
        );
    }
    io::Error::other(reason.to_string())
}

impl Drop for RuntimeChild {
    fn drop(&mut self) {
        if self.status.is_some()
            && self.output.complete()
            && self.output.error().is_none()
            && self.output.worker_stopped()
        {
            return;
        }

        let pid = self.id();
        let deadline = Instant::now() + DROP_REAP_BUDGET;
        let mut cleanup_error = None;
        if self.status.is_none() {
            match self.try_wait() {
                Ok(Some(_)) => {}
                Ok(None) => {
                    if let Err(error) = self.kill() {
                        cleanup_error = Some(format!("owned-child kill failed: {error}"));
                    }
                }
                Err(error) => {
                    let poll_error = format!("owned-child poll failed: {error}");
                    if let Err(kill_error) = self.kill() {
                        cleanup_error = Some(format!("{poll_error}; kill failed: {kill_error}"));
                    } else {
                        cleanup_error = Some(poll_error);
                    }
                }
            }
        }
        while Instant::now() < deadline {
            if self.status.is_none()
                && let Err(error) = self.try_wait()
            {
                cleanup_error = Some(format!("owned-child reap poll failed: {error}"));
                break;
            }
            if self.status.is_some() && self.output.complete() {
                break;
            }
            thread::sleep(POLL_QUANTUM.min(deadline.saturating_duration_since(Instant::now())));
        }
        if let Err(error) = self.output.stop() {
            cleanup_error = Some(format!("output-pump join failed: {error}"));
        }
        if self.status.is_none()
            || !self.output.complete()
            || self.output.truncated()
            || self.output.error().is_some()
        {
            eprintln!(
                "runtime test child cleanup incomplete: pid={pid}, reaped={}, {}, cleanup_error={}",
                self.status.is_some(),
                self.output.status_summary(),
                cleanup_error
                    .as_deref()
                    .unwrap_or("bounded cleanup deadline expired"),
            );
        }
    }
}
