// SPDX-License-Identifier: MIT

use std::io::Read;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const CHILD_TIMEOUT: Duration = Duration::from_secs(5);
const TERMINATION_TIMEOUT: Duration = Duration::from_secs(1);
const READER_JOIN_TIMEOUT: Duration = Duration::from_millis(250);
const MAX_CAPTURE_BYTES: usize = 64 * 1024;

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
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            let cleanup = terminate_child(&mut child);
            return Err(match cleanup {
                Ok(()) => String::from("runtime child did not expose stdout"),
                Err(error) => {
                    format!("runtime child did not expose stdout; child cleanup failed: {error}")
                }
            });
        }
    };
    let stderr = match child.stderr.take() {
        Some(stderr) => stderr,
        None => {
            let cleanup = terminate_child(&mut child);
            return Err(match cleanup {
                Ok(()) => String::from("runtime child did not expose stderr"),
                Err(error) => {
                    format!("runtime child did not expose stderr; child cleanup failed: {error}")
                }
            });
        }
    };
    let overflow = Arc::new(AtomicBool::new(false));
    let stdout_reader = spawn_reader(stdout, "stdout", Arc::clone(&overflow));
    let stderr_reader = spawn_reader(stderr, "stderr", Arc::clone(&overflow));
    let status = match monitor_child(&mut child, &overflow, timeout) {
        Ok(status) => status,
        Err(reason) => {
            let termination = terminate_child(&mut child);
            let stdout_result = join_reader_bounded(stdout_reader, "stdout");
            let stderr_result = join_reader_bounded(stderr_reader, "stderr");
            return match (termination, stdout_result, stderr_result) {
                (Err(cleanup), _, _) => Err(format!("{reason}; child cleanup failed: {cleanup}")),
                (_, Err(reader), _) | (_, _, Err(reader)) => Err(format!("{reason}; {reader}")),
                (Ok(()), Ok(_), Ok(_)) => Err(reason),
            };
        }
    };
    if let Err(cleanup) = terminate_child(&mut child) {
        let _ = join_reader_bounded(stdout_reader, "stdout");
        let _ = join_reader_bounded(stderr_reader, "stderr");
        return Err(format!("child cleanup failed: {cleanup}"));
    }
    let stdout = join_reader_bounded(stdout_reader, "stdout")?;
    let stderr = join_reader_bounded(stderr_reader, "stderr")?;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

fn spawn_reader<R>(
    mut reader: R,
    label: &'static str,
    overflow: Arc<AtomicBool>,
) -> JoinHandle<Result<Vec<u8>, String>>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut output = Vec::new();
        let mut buffer = [0_u8; 8 * 1024];
        loop {
            let count = reader
                .read(&mut buffer)
                .map_err(|error| format!("runtime child {label} read failed: {error}"))?;
            if count == 0 {
                return Ok(output);
            }
            let Some(new_length) = output.len().checked_add(count) else {
                overflow.store(true, Ordering::Release);
                return Err(format!(
                    "runtime child {label} output exceeded the capture bound"
                ));
            };
            if new_length > MAX_CAPTURE_BYTES {
                overflow.store(true, Ordering::Release);
                return Err(format!(
                    "runtime child {label} output exceeded the capture bound"
                ));
            }
            output.extend_from_slice(&buffer[..count]);
        }
    })
}

fn join_reader_bounded(
    reader: JoinHandle<Result<Vec<u8>, String>>,
    label: &'static str,
) -> Result<Vec<u8>, String> {
    let deadline = Instant::now() + READER_JOIN_TIMEOUT;
    while !reader.is_finished() {
        if Instant::now() >= deadline {
            drop(reader);
            return Err(format!(
                "runtime child {label} output reader did not finish after containment"
            ));
        }
        thread::sleep(Duration::from_millis(2));
    }
    match reader.join() {
        Ok(result) => result,
        Err(_) => Err(format!("runtime child {label} output reader panicked")),
    }
}

fn monitor_child(
    child: &mut Child,
    overflow: &AtomicBool,
    timeout: Duration,
) -> Result<ExitStatus, String> {
    let deadline = Instant::now() + timeout;
    loop {
        if overflow.load(Ordering::Acquire) {
            return Err(String::from(
                "runtime child output exceeded the capture bound",
            ));
        }
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if Instant::now() >= deadline => {
                return Err(String::from("runtime child exceeded its timeout"));
            }
            Ok(None) => thread::sleep(Duration::from_millis(5)),
            Err(error) => return Err(format!("cannot poll runtime child: {error}")),
        }
    }
}

fn terminate_child(child: &mut Child) -> Result<(), String> {
    let _group_exists = kill_owned_process_group(child)?;
    let _ = child.kill();
    let deadline = Instant::now() + TERMINATION_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => return Ok(()),
            Ok(None) if Instant::now() >= deadline => {
                return Err(String::from("runtime child did not exit after kill"));
            }
            Ok(None) => thread::sleep(Duration::from_millis(5)),
            Err(error) => return Err(format!("cannot reap runtime child: {error}")),
        }
    }
}

fn kill_owned_process_group(child: &Child) -> Result<bool, String> {
    let group_id = format!("-{}", child.id());
    let status = Command::new("/bin/kill")
        .arg("-KILL")
        .arg("--")
        .arg(group_id)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| format!("cannot invoke owned process-group cleanup: {error}"))?;
    Ok(status.success())
}
