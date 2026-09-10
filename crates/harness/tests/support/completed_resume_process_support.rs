// SPDX-License-Identifier: MIT

use std::io::{ErrorKind, Read};
use std::os::unix::process::CommandExt;
use std::process::{Child, ChildStderr, ChildStdout, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::fs::{OFlags, fcntl_getfl, fcntl_setfl};
use rustix::io::Errno;
use rustix::process::{Pid, Signal, WaitId, WaitIdOptions, kill_process_group, waitid};

const CHILD_TIMEOUT: Duration = Duration::from_secs(5);
const TERMINATION_TIMEOUT: Duration = Duration::from_secs(1);
const POLL_SLICE: Duration = Duration::from_millis(20);
const MAX_CAPTURE_BYTES: usize = 64 * 1024;
const MAX_DRAIN_READS: usize = 32;
const MAX_DRAIN_BYTES: usize = 64 * 1024;

pub(super) fn run_child(command: Command) -> Result<Output, String> {
    run_child_with_timeout(command, CHILD_TIMEOUT)
}

pub(super) fn run_child_with_timeout(
    mut command: Command,
    timeout: Duration,
) -> Result<Output, String> {
    command.process_group(0);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("cannot spawn runtime child: {error}"))?;
    let child_pid = Pid::from_child(&child);
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            return Err(abort_child(
                &mut child,
                child_pid,
                "runtime child did not expose stdout",
            ));
        }
    };
    let stderr = match child.stderr.take() {
        Some(stderr) => stderr,
        None => {
            return Err(abort_child(
                &mut child,
                child_pid,
                "runtime child did not expose stderr",
            ));
        }
    };
    if let Err(error) = set_nonblocking(&stdout, "stdout") {
        return Err(abort_child(
            &mut child,
            child_pid,
            &format!("{error}; runtime child stdout setup failed"),
        ));
    }
    if let Err(error) = set_nonblocking(&stderr, "stderr") {
        return Err(abort_child(
            &mut child,
            child_pid,
            &format!("{error}; runtime child stderr setup failed"),
        ));
    }
    let mut stdout = CaptureStream::new(stdout, "stdout");
    let mut stderr = CaptureStream::new(stderr, "stderr");
    let deadline = Instant::now() + timeout;
    let mut reason = None;
    let leader_exited = loop {
        match leader_exited_without_reap(child_pid) {
            Ok(true) => break true,
            Ok(false) => {}
            Err(error) => {
                reason = Some(format!("cannot inspect runtime child: {error}"));
                break false;
            }
        }
        if Instant::now() >= deadline {
            reason = Some(String::from("runtime child exceeded its timeout"));
            break false;
        }
        if let Err(error) = poll_and_drain(
            &mut stdout,
            &mut stderr,
            deadline.min(Instant::now() + POLL_SLICE),
        ) {
            reason = Some(error);
            break false;
        }
    };

    cleanup_child(
        &mut child,
        child_pid,
        leader_exited,
        reason,
        &mut stdout,
        &mut stderr,
    )
}

struct CaptureStream<R> {
    reader: R,
    bytes: Vec<u8>,
    open: bool,
    capture: bool,
    label: &'static str,
}

impl<R> CaptureStream<R> {
    fn new(reader: R, label: &'static str) -> Self {
        Self {
            reader,
            bytes: Vec::new(),
            open: true,
            capture: true,
            label,
        }
    }
}

fn set_nonblocking<R>(reader: &R, label: &'static str) -> Result<(), String>
where
    R: std::os::fd::AsFd,
{
    let flags = fcntl_getfl(reader)
        .map_err(|error| format!("cannot inspect runtime child {label} flags: {error}"))?;
    fcntl_setfl(reader, flags | OFlags::NONBLOCK)
        .map_err(|error| format!("cannot make runtime child {label} nonblocking: {error}"))
}

fn leader_exited_without_reap(pid: Pid) -> Result<bool, String> {
    let options = WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT;
    waitid(WaitId::Pid(pid), options)
        .map(|status| status.is_some())
        .map_err(|error| format!("waitid failed: {error}"))
}

fn poll_and_drain(
    stdout: &mut CaptureStream<ChildStdout>,
    stderr: &mut CaptureStream<ChildStderr>,
    deadline: Instant,
) -> Result<(), String> {
    drain_stream(stdout)?;
    drain_stream(stderr)?;
    if !stdout.open && !stderr.open {
        thread::sleep(Duration::from_millis(2));
        return Ok(());
    }

    let mut descriptors = Vec::with_capacity(2);
    let stdout_index = if stdout.open {
        let index = descriptors.len();
        descriptors.push(PollFd::new(
            &stdout.reader,
            PollFlags::IN | PollFlags::HUP | PollFlags::ERR,
        ));
        Some(index)
    } else {
        None
    };
    let stderr_index = if stderr.open {
        let index = descriptors.len();
        descriptors.push(PollFd::new(
            &stderr.reader,
            PollFlags::IN | PollFlags::HUP | PollFlags::ERR,
        ));
        Some(index)
    } else {
        None
    };
    let timeout = poll_timeout(deadline);
    let polled = match poll(&mut descriptors, Some(&timeout)) {
        Ok(count) => count,
        Err(error) if error == Errno::INTR => 0,
        Err(error) => return Err(format!("runtime child output poll failed: {error}")),
    };
    let stdout_ready = stdout_index.is_some_and(|index| {
        descriptors[index]
            .revents()
            .intersects(PollFlags::IN | PollFlags::HUP | PollFlags::ERR)
    });
    let stderr_ready = stderr_index.is_some_and(|index| {
        descriptors[index]
            .revents()
            .intersects(PollFlags::IN | PollFlags::HUP | PollFlags::ERR)
    });
    drop(descriptors);
    if polled == 0 {
        return Ok(());
    }
    if stdout_ready {
        drain_stream(stdout)?;
    }
    if stderr_ready {
        drain_stream(stderr)?;
    }
    Ok(())
}

fn drain_stream<R>(stream: &mut CaptureStream<R>) -> Result<(), String>
where
    R: Read,
{
    let mut buffer = [0_u8; 8 * 1024];
    let mut reads = 0;
    let mut bytes = 0;
    while reads < MAX_DRAIN_READS && bytes < MAX_DRAIN_BYTES {
        reads += 1;
        match stream.reader.read(&mut buffer) {
            Ok(0) => {
                stream.open = false;
                return Ok(());
            }
            Ok(count) => {
                bytes += count;
                if !stream.capture {
                    continue;
                }
                let Some(new_length) = stream.bytes.len().checked_add(count) else {
                    stream.capture = false;
                    return Err(format!(
                        "runtime child {} output exceeded the capture bound",
                        stream.label
                    ));
                };
                if new_length > MAX_CAPTURE_BYTES {
                    stream.capture = false;
                    return Err(format!(
                        "runtime child {} output exceeded the capture bound",
                        stream.label
                    ));
                }
                stream.bytes.extend_from_slice(&buffer[..count]);
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == ErrorKind::WouldBlock => return Ok(()),
            Err(error) => {
                stream.open = false;
                return Err(format!(
                    "runtime child {} read failed: {error}",
                    stream.label
                ));
            }
        }
    }
    Ok(())
}

fn poll_timeout(deadline: Instant) -> Timespec {
    let remaining = deadline.saturating_duration_since(Instant::now());
    Timespec {
        tv_sec: remaining.as_secs().min(i64::MAX as u64) as i64,
        tv_nsec: remaining.subsec_nanos() as _,
    }
}

fn abort_child(child: &mut Child, child_pid: Pid, reason: &str) -> String {
    let mut errors = Vec::new();
    if let Err(error) = kill_owned_process_group(child_pid, false) {
        errors.push(error);
    }
    if let Err(error) = child.kill() {
        errors.push(format!("runtime child direct kill failed: {error}"));
    }
    let deadline = Instant::now() + TERMINATION_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => break,
            Ok(None) if Instant::now() >= deadline => {
                errors.push(String::from(
                    "runtime child did not reap after setup failure",
                ));
                break;
            }
            Ok(None) => thread::sleep(Duration::from_millis(2)),
            Err(error) => {
                errors.push(format!("cannot reap runtime child: {error}"));
                break;
            }
        }
    }
    if errors.is_empty() {
        reason.to_owned()
    } else {
        format!("{reason}; {}", errors.join("; "))
    }
}

fn cleanup_child(
    child: &mut Child,
    child_pid: Pid,
    leader_exited: bool,
    reason: Option<String>,
    stdout: &mut CaptureStream<ChildStdout>,
    stderr: &mut CaptureStream<ChildStderr>,
) -> Result<Output, String> {
    let mut cleanup_error = kill_owned_process_group(child_pid, leader_exited).err();
    if let Err(error) = child.kill() {
        let expected_dead_child =
            leader_exited && matches!(error.kind(), ErrorKind::InvalidInput | ErrorKind::NotFound);
        if !expected_dead_child && cleanup_error.is_none() {
            cleanup_error = Some(format!("runtime child direct kill failed: {error}"));
        }
    }

    let deadline = Instant::now() + TERMINATION_TIMEOUT;
    let mut status = None;
    loop {
        if stdout.open {
            retain_first_error(&mut cleanup_error, drain_stream(stdout));
        }
        if stderr.open {
            retain_first_error(&mut cleanup_error, drain_stream(stderr));
        }
        if status.is_none() {
            match child.try_wait() {
                Ok(Some(child_status)) => status = Some(child_status),
                Ok(None) => {}
                Err(error) => {
                    if cleanup_error.is_none() {
                        cleanup_error = Some(format!("cannot reap runtime child: {error}"));
                    }
                }
            }
        }
        if status.is_some() && !stdout.open && !stderr.open {
            break;
        }
        if Instant::now() >= deadline {
            if cleanup_error.is_none() {
                cleanup_error = Some(String::from(
                    "runtime child cleanup did not finish before its deadline",
                ));
            }
            break;
        }
        retain_first_error(
            &mut cleanup_error,
            poll_and_drain(stdout, stderr, deadline.min(Instant::now() + POLL_SLICE)),
        );
    }

    let Some(status) = status else {
        return Err(cleanup_error
            .unwrap_or_else(|| String::from("runtime child was not reaped after cleanup")));
    };
    if let Some(error) = cleanup_error {
        return Err(match reason {
            Some(reason) => format!("{reason}; {error}"),
            None => error,
        });
    }
    if let Some(reason) = reason {
        return Err(reason);
    }
    Ok(Output {
        status,
        stdout: std::mem::take(&mut stdout.bytes),
        stderr: std::mem::take(&mut stderr.bytes),
    })
}

fn retain_first_error(slot: &mut Option<String>, result: Result<(), String>) {
    if slot.is_none() {
        *slot = result.err();
    }
}

fn kill_owned_process_group(pid: Pid, leader_exited: bool) -> Result<(), String> {
    match kill_process_group(pid, Signal::KILL) {
        Ok(()) => Ok(()),
        Err(error) if error == Errno::SRCH && leader_exited => Ok(()),
        Err(error) => Err(format!(
            "owned process-group cleanup failed for {pid}: {error}"
        )),
    }
}

#[cfg(test)]
#[path = "completed_resume_drain_tests.rs"]
mod tests;
