// SPDX-License-Identifier: MIT

use std::io::Read;
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const CHILD_TIMEOUT: Duration = Duration::from_secs(5);
const TERMINATION_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_CAPTURE_BYTES: usize = 64 * 1024;

pub(super) fn run_child(mut command: Command) -> Result<Output, String> {
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
    let status = match monitor_child(&mut child, &overflow) {
        Ok(status) => status,
        Err(reason) => {
            let termination = terminate_child(&mut child);
            if let Err(cleanup) = termination {
                return Err(format!("{reason}; child cleanup failed: {cleanup}"));
            }
            let _ = join_reader(stdout_reader);
            let _ = join_reader(stderr_reader);
            return Err(reason);
        }
    };
    let stdout = join_reader(stdout_reader)?;
    let stderr = join_reader(stderr_reader)?;
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

fn join_reader(reader: JoinHandle<Result<Vec<u8>, String>>) -> Result<Vec<u8>, String> {
    match reader.join() {
        Ok(result) => result,
        Err(_) => Err(String::from("runtime child output reader panicked")),
    }
}

fn monitor_child(child: &mut Child, overflow: &AtomicBool) -> Result<ExitStatus, String> {
    let deadline = Instant::now() + CHILD_TIMEOUT;
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
