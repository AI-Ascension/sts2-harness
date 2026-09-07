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
const PROCESS_GROUP_COMMAND_TIMEOUT: Duration = Duration::from_millis(250);
const PROCESS_GROUP_COMMAND_REAP_TIMEOUT: Duration = Duration::from_millis(250);
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
    let child_id = child.id();
    let child_reaped = child
        .try_wait()
        .map_err(|error| format!("cannot inspect runtime child before cleanup: {error}"))?
        .is_some();
    kill_owned_process_group(child, child_id, child_reaped)?;
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

fn kill_owned_process_group(
    child: &mut Child,
    child_id: u32,
    mut child_reaped: bool,
) -> Result<(), String> {
    let group_id = format!("-{child_id}");
    let group_probe =
        run_process_group_command("-0", &group_id, "owned process-group identity probe")?;
    if !group_probe.success() {
        if !child_reaped {
            child_reaped = child
                .try_wait()
                .map_err(|error| {
                    format!("cannot inspect runtime child after group probe: {error}")
                })?
                .is_some();
        }
        if child_reaped {
            return Ok(());
        }
        return Err(format!(
            "owned process-group identity probe returned {group_probe:?} while the direct child remained"
        ));
    }

    let status = run_process_group_command("-KILL", &group_id, "owned process-group cleanup")?;
    if status.success() {
        return Ok(());
    }

    let probe = run_process_group_command("-0", &group_id, "owned process-group existence probe")?;
    if !probe.success() && child_reaped {
        // try_wait has already reaped the direct child. A nonzero KILL plus a
        // nonzero zero-signal probe is the expected no-process-group case.
        return Ok(());
    }
    if !probe.success() {
        return Err(format!(
            "owned process-group cleanup returned {status:?} and its group disappeared before the direct child was reaped"
        ));
    }
    Err(format!(
        "owned process-group cleanup returned {status:?} while its process group remained"
    ))
}

fn run_process_group_command(
    signal: &str,
    group_id: &str,
    label: &str,
) -> Result<ExitStatus, String> {
    let mut command = Command::new("/bin/kill");
    command
        .env_clear()
        .arg(signal)
        .arg("--")
        .arg(group_id)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut process = command
        .spawn()
        .map_err(|error| format!("cannot invoke {label}: {error}"))?;
    wait_command_bounded(&mut process, PROCESS_GROUP_COMMAND_TIMEOUT, label)
}

fn wait_command_bounded(
    process: &mut Child,
    timeout: Duration,
    label: &str,
) -> Result<ExitStatus, String> {
    let deadline = Instant::now() + timeout;
    loop {
        match process.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if Instant::now() >= deadline => {
                let _ = process.kill();
                let reap_deadline = Instant::now() + PROCESS_GROUP_COMMAND_REAP_TIMEOUT;
                loop {
                    match process.try_wait() {
                        Ok(Some(_status)) => {
                            return Err(format!("{label} exceeded its deadline"));
                        }
                        Ok(None) if Instant::now() >= reap_deadline => {
                            return Err(format!(
                                "{label} exceeded its deadline and could not be reaped"
                            ));
                        }
                        Ok(None) => thread::sleep(Duration::from_millis(2)),
                        Err(error) => {
                            return Err(format!(
                                "{label} exceeded its deadline; reaping failed: {error}"
                            ));
                        }
                    }
                }
            }
            Ok(None) => thread::sleep(Duration::from_millis(2)),
            Err(error) => return Err(format!("cannot poll {label}: {error}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_command_wait_is_bounded() -> Result<(), String> {
        let mut command = Command::new("/bin/sh");
        command
            .env_clear()
            .arg("-c")
            .arg("sleep 30")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut process = command
            .spawn()
            .map_err(|error| format!("cannot spawn cleanup command fixture: {error}"))?;
        let started = Instant::now();
        let result = wait_command_bounded(
            &mut process,
            Duration::from_millis(20),
            "cleanup command fixture",
        );
        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(
            process
                .try_wait()
                .map_err(|error| format!("cannot reap cleanup command fixture: {error}"))?
                .is_some()
        );
        Ok(())
    }
}
